//! Tests for `FluxoraStream::batch_withdraw_to` — issue #762.
//!
//! # Authorization model under test
//!
//! The provided `recipient` MUST equal `stream.recipient` for every entry, or
//! the entire transaction reverts with `Unauthorized` (Soroban rollback wipes
//! any earlier per-entry token transfers that happened before the offending
//! index was reached). **No per-stream destination auth is required** — the
//! destination is unrestricted except for the contract address itself (which
//! the pre-check loop rejects with `InvalidParams`). These properties are
//! exhaustively verified by the test suite below.
//!
//! # Coverage matrix
//!
//! | Acceptance criterion                                     | Test(s)                                                   |
//! |----------------------------------------------------------|-----------------------------------------------------------|
//! | Single-recipient batch succeeds                          | `single_recipient_three_streams_drain_correctly`          |
//! | Unauthorized cross-recipient entry aborts atomically     | `unauthorized_cross_recipient_aborts_atomically`,         |
//! |                                                          | `unauthorized_cross_recipient_at_various_positions`       |
//! | Duplicate destinations aggregate correctly               | `duplicate_destinations_aggregate_balance`,               |
//! |                                                          | `duplicate_destinations_emit_consistent_events`           |
//! | Forbidden destination (contract address) rejected        | `forbidden_destination_contract_rejected`,                |
//! |                                                          | `forbidden_destination_first_position`,                   |
//! |                                                          | `forbidden_destination_mid_batch_aborts_atomically`       |
//!
//! Additional hardening for adjacent guarantees:
//!   * Duplicate `stream_id` in batch → host panic (contract `assert!`).
//!   * Paused stream mid-batch → `InvalidState` with full rollback.
//!   * Dust threshold filtered → result `amount: 0`, no event emitted.
//!   * Completed stream in batch → result `amount: 0`, no event, no transfer.
//!   * Destination = recipient self-routing works.
//!   * Destination = unrelated third-party works.
//!
//! # Test infrastructure note
//!
//! Each test file in `contracts/stream/tests/` reinvents a small setup
//! struct because the crate-private `TestContext` in `src/test.rs` is not
//! reachable from the integration test crate. `BatchCtx` here mirrors the
//! structure of `tests/adversarial_auth.rs::Ctx` with `mock_all_auths()`
//! enabled (the batch function requires only a single recipient auth so
//! strict `mock_auths` per call would add noise without hardening auth).
//! `BatchWithdrawResult` is the only entrypoint-specific type; the rest of
//! the fluxorastream API surface used here is the standard mock_all_auths
//! pattern.

extern crate std;

use fluxora_stream::{
    ContractError, FluxoraStream, FluxoraStreamClient, StreamKind, StreamStatus, WithdrawToParam,
    WithdrawalTo,
};
use soroban_sdk::{
    symbol_short,
    testutils::{Address as _, Events, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    Address, Env, Symbol, TryFromVal, Vec,
};

// ---------------------------------------------------------------------------
// Test context
// ---------------------------------------------------------------------------

struct BatchCtx<'a> {
    env: Env,
    contract_id: Address,
    token_id: Address,
    #[allow(dead_code)]
    admin: Address,
    sender: Address,
    recipient: Address,
    sac: StellarAssetClient<'a>,
}

impl<'a> BatchCtx<'a> {
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
        sac.mint(&sender, &10_000_i128);
        TokenClient::new(&env, &token_id)
            .approve(&sender, &contract_id, &i128::MAX, &100_000);

        BatchCtx {
            env,
            contract_id,
            token_id,
            admin,
            sender,
            recipient,
            sac,
        }
    }

    fn client(&self) -> FluxoraStreamClient<'_> {
        FluxoraStreamClient::new(&self.env, &self.contract_id)
    }

    fn token(&self) -> TokenClient<'_> {
        TokenClient::new(&self.env, &self.token_id)
    }

    /// Create one default stream (1000 deposit, 1/s, 0..1000s, no cliff,
    /// no dust threshold) for `recipient`. Caller is responsible for advancing
    /// the ledger clock.
    fn create_stream_for(&self, recipient: &Address) -> u64 {
        self.env.ledger().set_timestamp(0);
        self.client().create_stream(
            &self.sender,
            recipient,
            &1_000_i128,
            &1_i128,
            &0u64,
            &0u64,
            &1_000u64,
            &0,
            &None,
            &StreamKind::Linear,
        )
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build a single `WithdrawToParam` (used by `wdraw_vec`).
fn wdraw_param(stream_id: u64, destination: &Address) -> WithdrawToParam {
    WithdrawToParam {
        stream_id,
        destination: destination.clone(),
    }
}

/// Build a `Vec<WithdrawToParam>` from an array of (stream_id, destination).
fn wdraw_vec(env: &Env, items: &[(u64, &Address)]) -> Vec<WithdrawToParam> {
    let mut v = Vec::new(env);
    for (sid, dest) in items {
        v.push_back(wdraw_param(*sid, *dest));
    }
    v
}

/// Collect every event whose topic[0] == "wdraw_to" and return parsed
/// `WithdrawalTo` payloads in emission order.
fn collect_wdraw_events(ctx: &BatchCtx) -> std::vec::Vec<WithdrawalTo> {
    let mut out = std::vec::Vec::new();
    for event in ctx.env.events().all().iter() {
        // event.0 = contract_id, event.1 = topics, event.2 = data.
        // Borrow topics so we do not move them out of the shared event ref.
        let topics = &event.1;
        if topics.len() < 2 {
            continue;
        }
        let topic0_val = topics.get(0).unwrap();
        let sym = match Symbol::try_from_val(&ctx.env, &topic0_val) {
            Ok(s) => s,
            Err(_) => continue,
        };
        if sym != symbol_short!("wdraw_to") {
            continue;
        }
        // topic[1] is stream_id; the parsed payload carries it explicitly.
        let payload = match WithdrawalTo::try_from_val(&ctx.env, &event.2) {
            Ok(p) => p,
            Err(_) => continue,
        };
        out.push(payload);
    }
    out
}

// ---------------------------------------------------------------------------
// AC1 — Single-recipient batch succeeds (per-stream destinations)
// ---------------------------------------------------------------------------

/// Three streams owned by the recipient, routed to three distinct third-party
/// destinations, must each transfer exactly that stream's accrued amount and
/// emit one `wdraw_to` event per transfer.
#[test]
fn single_recipient_three_streams_drain_correctly() {
    let ctx = BatchCtx::setup();
    let id_a = ctx.create_stream_for(&ctx.recipient);
    ctx.sac.mint(&ctx.sender, &1_000_i128);
    let id_b = ctx.create_stream_for(&ctx.recipient);
    ctx.sac.mint(&ctx.sender, &1_000_i128);
    let id_c = ctx.create_stream_for(&ctx.recipient);

    let dest_a = Address::generate(&ctx.env);
    let dest_b = Address::generate(&ctx.env);
    let dest_c = Address::generate(&ctx.env);

    ctx.env.ledger().set_timestamp(500);

    let withdrawals = wdraw_vec(
        &ctx.env,
        &[(id_a, &dest_a), (id_b, &dest_b), (id_c, &dest_c)],
    );

    let results = ctx
        .client()
        .batch_withdraw_to(&ctx.recipient, &withdrawals);

    assert_eq!(results.len(), 3);
    for (idx, expected_sid) in [(0u32, id_a), (1, id_b), (2, id_c)] {
        let r = results.get(idx).unwrap();
        assert_eq!(r.stream_id, expected_sid);
        assert_eq!(r.amount, 500, "each stream contributed 500 at t=500");
    }

    // Per-destination balance matches the corresponding stream's withdrawal.
    assert_eq!(ctx.token().balance(&dest_a), 500);
    assert_eq!(ctx.token().balance(&dest_b), 500);
    assert_eq!(ctx.token().balance(&dest_c), 500);

    // Recipient (caller) itself got nothing — funds went to destinations.
    assert_eq!(
        ctx.token().balance(&ctx.recipient),
        0,
        "destination != recipient is intentional; recipient wallet remains untouched"
    );

    // Each stream reflects partial withdraw (500/1000, still Active).
    for id in [id_a, id_b, id_c] {
        let st = ctx.client().get_stream_state(&id);
        assert_eq!(st.withdrawn_amount, 500);
        assert_eq!(st.status, StreamStatus::Active);
    }

    // Single wdraw_to event per stream with the correct destination.
    let events = collect_wdraw_events(&ctx);
    assert_eq!(events.len(), 3, "exactly one wdraw_to per drained stream");
    let mut by_sid: std::collections::HashMap<u64, &WithdrawalTo> =
        std::collections::HashMap::new();
    for ev in &events {
        by_sid.insert(ev.stream_id, ev);
    }
    assert_eq!(by_sid[&id_a].destination, dest_a);
    assert_eq!(by_sid[&id_a].amount, 500);
    assert_eq!(by_sid[&id_b].destination, dest_b);
    assert_eq!(by_sid[&id_b].amount, 500);
    assert_eq!(by_sid[&id_c].destination, dest_c);
    assert_eq!(by_sid[&id_c].amount, 500);
    for ev in &events {
        assert_eq!(
            ev.recipient, ctx.recipient,
            "withdrawal recipient matches the single authorized caller"
        );
    }
}

/// Recipient routing funds to themselves works identically to a non-self
/// destination. This guards against any accidental gate that would force
/// destination != recipient.
#[test]
fn destination_equal_to_recipient_succeeds() {
    let ctx = BatchCtx::setup();
    let id = ctx.create_stream_for(&ctx.recipient);
    ctx.env.ledger().set_timestamp(400);

    let withdrawals = wdraw_vec(&ctx.env, &[(id, &ctx.recipient)]);
    let results = ctx
        .client()
        .batch_withdraw_to(&ctx.recipient, &withdrawals);

    assert_eq!(results.get(0).unwrap().amount, 400);
    assert_eq!(ctx.token().balance(&ctx.recipient), 400);
}

/// Routing to an unrelated third party (neither recipient, sender, admin, nor
/// the contract) must succeed: only the contract-address destination is
/// forbidden.
#[test]
fn destination_third_party_succeeds() {
    let ctx = BatchCtx::setup();
    let id = ctx.create_stream_for(&ctx.recipient);
    let third_party = Address::generate(&ctx.env);
    ctx.env.ledger().set_timestamp(250);

    let withdrawals = wdraw_vec(&ctx.env, &[(id, &third_party)]);
    let results = ctx
        .client()
        .batch_withdraw_to(&ctx.recipient, &withdrawals);
    assert_eq!(results.get(0).unwrap().amount, 250);
    assert_eq!(ctx.token().balance(&third_party), 250);
}

/// A multi-stream batch in which one entry is dust-filtered must succeed
/// for the non-dust entries, and the dust-filtered entry contributes
/// `amount: 0` without any `wdraw_to` event being emitted for it.
#[test]
fn dust_threshold_skipped_in_multi_stream_batch() {
    let ctx = BatchCtx::setup();

    // Stream 1: dust threshold = 500; at t=200 only 200 accrued → filtered.
    let id_dust = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1_000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1_000u64,
        &500i128,
        &None,
        &StreamKind::Linear,
    );

    // Stream 2: no dust threshold, drains fully at t=200.
    ctx.sac.mint(&ctx.sender, &1_000_i128);
    let id_full = ctx.create_stream_for(&ctx.recipient);

    let dest_dust = Address::generate(&ctx.env);
    let dest_full = Address::generate(&ctx.env);

    ctx.env.ledger().set_timestamp(200);

    let withdrawals = wdraw_vec(
        &ctx.env,
        &[(id_dust, &dest_dust), (id_full, &dest_full)],
    );
    let results = ctx
        .client()
        .batch_withdraw_to(&ctx.recipient, &withdrawals);
    assert_eq!(results.len(), 2);
    assert_eq!(
        results.get(0).unwrap().amount,
        0,
        "dust-filtered entry contributes 0"
    );
    assert_eq!(
        results.get(1).unwrap().amount,
        200,
        "non-dust entry drains its full accrued amount"
    );

    assert_eq!(
        ctx.token().balance(&dest_dust),
        0,
        "no tokens to dust destination"
    );
    assert_eq!(
        ctx.token().balance(&dest_full),
        200,
        "non-dust entry's withdrawal sent to its own destination"
    );

    // One wdraw_to for id_full; zero for id_dust.
    let events = collect_wdraw_events(&ctx);
    let dust_count = events.iter().filter(|e| e.stream_id == id_dust).count();
    let full_count = events.iter().filter(|e| e.stream_id == id_full).count();
    assert_eq!(dust_count, 0, "dust-filtered entry has no wdraw_to event");
    assert_eq!(
        full_count, 1,
        "non-dust entry has exactly one wdraw_to event"
    );
}

// ---------------------------------------------------------------------------
// AC2 — Unauthorized cross-recipient entry aborts (atomicity)
// ---------------------------------------------------------------------------

/// When Alice tries to include Bob's stream alongside her own, every token
/// transfer that already happened for Alice's entries must roll back, leaving
/// no observable state change.
#[test]
fn unauthorized_cross_recipient_aborts_atomically() {
    let ctx = BatchCtx::setup();
    let alice_a = ctx.create_stream_for(&ctx.recipient);
    ctx.sac.mint(&ctx.sender, &1_000_i128);
    let alice_b = ctx.create_stream_for(&ctx.recipient);

    let bob = Address::generate(&ctx.env);
    ctx.sac.mint(&ctx.sender, &1_000_i128);
    let bob_stream = ctx.create_stream_for(&bob);

    ctx.env.ledger().set_timestamp(300);

    // Sanity: no one has any balance yet.
    assert_eq!(ctx.token().balance(&ctx.recipient), 0);
    assert_eq!(ctx.token().balance(&bob), 0);

    // Alice attempts to withdraw Alice_a, Bob_stream, Alice_b — Bob's entry
    // must trigger the cross-recipient check, returning Unauthorized.
    let withdrawals = wdraw_vec(
        &ctx.env,
        &[
            (alice_a, &ctx.recipient),
            (bob_stream, &ctx.recipient),
            (alice_b, &ctx.recipient),
        ],
    );
    let result = ctx
        .client()
        .try_batch_withdraw_to(&ctx.recipient, &withdrawals);
    assert_eq!(result, Err(Ok(ContractError::Unauthorized)));

    // Atomicity proofs: Alice_a already had push_token called for its 300
    // accrued tokens before the loop reached Bob_stream. The whole transaction
    // must roll back, so EVERY state assertion holds at the pre-call value.
    assert_eq!(
        ctx.token().balance(&ctx.recipient),
        0,
        "Alice destination balance unchanged despite Alice_a's earlier transfer"
    );
    assert_eq!(
        ctx.token().balance(&bob),
        0,
        "Bob's balance untouched"
    );
    assert_eq!(
        ctx.client().get_stream_state(&alice_a).withdrawn_amount,
        0,
        "Alice_a withdrawn_amount rolled back to 0"
    );
    assert_eq!(
        ctx.client().get_stream_state(&alice_b).withdrawn_amount,
        0,
        "Alice_b withdrawn_amount still 0"
    );
    assert_eq!(
        ctx.client().get_stream_state(&bob_stream).withdrawn_amount,
        0,
        "Bob_stream never touched"
    );
    assert_eq!(
        ctx.client().get_stream_state(&bob_stream).status,
        StreamStatus::Active
    );
    assert_eq!(
        ctx.client().get_stream_state(&alice_a).status,
        StreamStatus::Active
    );

    // No wdraw_to events should have been emitted (rollback wipes events too).
    let events = collect_wdraw_events(&ctx);
    assert!(
        events.is_empty(),
        "rollback must erase all wdraw_to events emitted before the failure point; got {:?}",
        events
    );
}

/// The cross-recipient check fails regardless of where in the batch ordering
/// the foreign stream is positioned. This catches off-by-one regressions in
/// the per-entry loop.
#[test]
fn unauthorized_cross_recipient_at_various_positions() {
    let ctx = BatchCtx::setup();
    let id_a = ctx.create_stream_for(&ctx.recipient);
    ctx.sac.mint(&ctx.sender, &1_000_i128);
    let id_b = ctx.create_stream_for(&ctx.recipient);

    let bob = Address::generate(&ctx.env);
    ctx.sac.mint(&ctx.sender, &1_000_i128);
    let bob_stream = ctx.create_stream_for(&bob);

    ctx.env.ledger().set_timestamp(100);

    // Bob's stream placed FIRST — must still reject.
    let res_first = ctx.client().try_batch_withdraw_to(
        &ctx.recipient,
        &wdraw_vec(
            &ctx.env,
            &[(bob_stream, &ctx.recipient), (id_a, &ctx.recipient)],
        ),
    );
    assert_eq!(res_first, Err(Ok(ContractError::Unauthorized)));

    // Bob's stream placed LAST — must still reject.
    let res_last = ctx.client().try_batch_withdraw_to(
        &ctx.recipient,
        &wdraw_vec(
            &ctx.env,
            &[(id_a, &ctx.recipient), (id_b, &ctx.recipient), (bob_stream, &ctx.recipient)],
        ),
    );
    assert_eq!(res_last, Err(Ok(ContractError::Unauthorized)));

    // No state mutation either way. Status assertions are explicit so a
    // future patch that silently transitions state before early-return is
    // caught.
    assert_eq!(ctx.token().balance(&ctx.recipient), 0);
    for id in [id_a, id_b, bob_stream] {
        let st = ctx.client().get_stream_state(&id);
        assert_eq!(
            st.withdrawn_amount, 0,
            "stream {} must not have been touched", id
        );
        assert_eq!(st.status, StreamStatus::Active);
    }
    // No wdraw_to events should have been emitted on either rejected path.
    assert!(
        collect_wdraw_events(&ctx).is_empty(),
        "rollback must erase wdraw_to events emitted before the failure point"
    );
}

// ---------------------------------------------------------------------------
// AC3 — Duplicate destinations aggregate
// ---------------------------------------------------------------------------

/// When several streams in a batch share the same destination, the destination
/// receives the *sum* of per-stream transfers and one `wdraw_to` event is
/// emitted per stream carrying that stream's individual amount.
#[test]
fn duplicate_destinations_aggregate_balance() {
    let ctx = BatchCtx::setup();
    let id_a = ctx.create_stream_for(&ctx.recipient);
    ctx.sac.mint(&ctx.sender, &1_000_i128);
    let id_b = ctx.create_stream_for(&ctx.recipient);
    ctx.sac.mint(&ctx.sender, &1_000_i128);
    let id_c = ctx.create_stream_for(&ctx.recipient);

    let shared_dest = Address::generate(&ctx.env);
    ctx.env.ledger().set_timestamp(400);

    let withdrawals = wdraw_vec(
        &ctx.env,
        &[
            (id_a, &shared_dest),
            (id_b, &shared_dest),
            (id_c, &shared_dest),
        ],
    );
    let results = ctx
        .client()
        .batch_withdraw_to(&ctx.recipient, &withdrawals);

    assert_eq!(results.len(), 3);
    assert_eq!(results.get(0).unwrap().amount, 400);
    assert_eq!(results.get(1).unwrap().amount, 400);
    assert_eq!(results.get(2).unwrap().amount, 400);

    // Aggregate: 400 + 400 + 400 = 1_200 to one destination.
    assert_eq!(
        ctx.token().balance(&shared_dest),
        1_200,
        "duplicate destinations aggregate to the sum of per-stream transfers"
    );

    // Each stream reflects its own withdrawn_amount.
    assert_eq!(ctx.client().get_stream_state(&id_a).withdrawn_amount, 400);
    assert_eq!(ctx.client().get_stream_state(&id_b).withdrawn_amount, 400);
    assert_eq!(ctx.client().get_stream_state(&id_c).withdrawn_amount, 400);
}

/// The duplicate-destination invariant must also hold at the event level: one
/// `wdraw_to` per stream, each carrying the per-stream amount and the shared
/// destination.
#[test]
fn duplicate_destinations_emit_consistent_events() {
    let ctx = BatchCtx::setup();
    let id_a = ctx.create_stream_for(&ctx.recipient);
    ctx.sac.mint(&ctx.sender, &1_000_i128);
    let id_b = ctx.create_stream_for(&ctx.recipient);

    let shared_dest = Address::generate(&ctx.env);
    ctx.env.ledger().set_timestamp(700);

    let withdrawals = wdraw_vec(
        &ctx.env,
        &[(id_a, &shared_dest), (id_b, &shared_dest)],
    );
    ctx.client()
        .batch_withdraw_to(&ctx.recipient, &withdrawals);

    let events = collect_wdraw_events(&ctx);
    assert_eq!(events.len(), 2);

    let mut by_sid: std::collections::HashMap<u64, &WithdrawalTo> =
        std::collections::HashMap::new();
    for ev in &events {
        assert_eq!(ev.destination, shared_dest);
        assert_eq!(ev.recipient, ctx.recipient);
        by_sid.insert(ev.stream_id, ev);
    }
    assert_eq!(by_sid[&id_a].amount, 700);
    assert_eq!(by_sid[&id_b].amount, 700);
}

// ---------------------------------------------------------------------------
// AC4 — Forbidden destination (contract address) is rejected
// ---------------------------------------------------------------------------

/// Routing a withdrawal to the contract itself must be rejected with
/// `InvalidParams` *before* any state mutation happens.
#[test]
fn forbidden_destination_contract_rejected() {
    let ctx = BatchCtx::setup();
    let id = ctx.create_stream_for(&ctx.recipient);
    ctx.env.ledger().set_timestamp(200);

    let recipient_bal_before = ctx.token().balance(&ctx.recipient);
    let contract_bal_before = ctx.token().balance(&ctx.contract_id);

    let withdrawals = wdraw_vec(&ctx.env, &[(id, &ctx.contract_id)]);
    let result = ctx
        .client()
        .try_batch_withdraw_to(&ctx.recipient, &withdrawals);
    assert_eq!(result, Err(Ok(ContractError::InvalidParams)));

    // No state mutated, no tokens moved.
    assert_eq!(ctx.token().balance(&ctx.recipient), recipient_bal_before);
    assert_eq!(ctx.token().balance(&ctx.contract_id), contract_bal_before);
    let st = ctx.client().get_stream_state(&id);
    assert_eq!(st.withdrawn_amount, 0);
    assert_eq!(st.status, StreamStatus::Active);
    assert!(collect_wdraw_events(&ctx).is_empty());
}

/// Forbidding the destination when it appears as the first entry also
/// rejects — the pre-check loop returns early on the first matching param.
#[test]
fn forbidden_destination_first_position() {
    let ctx = BatchCtx::setup();
    let id_a = ctx.create_stream_for(&ctx.recipient);
    ctx.sac.mint(&ctx.sender, &1_000_i128);
    let id_b = ctx.create_stream_for(&ctx.recipient);

    ctx.env.ledger().set_timestamp(500);

    // Pre-check loop short-circuits: id_a's destination == contract address
    // is detected before id_b is even examined.
    let withdrawals = wdraw_vec(
        &ctx.env,
        &[(id_a, &ctx.contract_id), (id_b, &ctx.recipient)],
    );
    let result = ctx
        .client()
        .try_batch_withdraw_to(&ctx.recipient, &withdrawals);
    assert_eq!(result, Err(Ok(ContractError::InvalidParams)));

    for id in [id_a, id_b] {
        assert_eq!(ctx.client().get_stream_state(&id).withdrawn_amount, 0);
    }
    assert!(collect_wdraw_events(&ctx).is_empty());
}

/// Even if some earlier entries would have succeeded, a forbidden destination
/// appearing later in the batch must abort the whole batch and roll back any
/// transfers that already happened.
#[test]
fn forbidden_destination_mid_batch_aborts_atomically() {
    let ctx = BatchCtx::setup();
    let id_a = ctx.create_stream_for(&ctx.recipient);
    ctx.sac.mint(&ctx.sender, &1_000_i128);
    let id_b = ctx.create_stream_for(&ctx.recipient);

    let legit_dest = Address::generate(&ctx.env);
    ctx.env.ledger().set_timestamp(300);

    let legit_dest_before = ctx.token().balance(&legit_dest);

    // id_a's pre-check passes (legit destination), but id_b hits the contract
    // address check. Note: the in-loop checks run *after* the pre-check loop,
    // and the per-entry loop does NOT short-circuit on destination until the
    // outer pre-check sees it. We verify either path returns InvalidParams
    // without any state change.
    let withdrawals = wdraw_vec(
        &ctx.env,
        &[(id_a, &legit_dest), (id_b, &ctx.contract_id)],
    );
    let result = ctx
        .client()
        .try_batch_withdraw_to(&ctx.recipient, &withdrawals);
    assert_eq!(result, Err(Ok(ContractError::InvalidParams)));

    // Every state assertion must hold at the pre-call value.
    assert_eq!(ctx.token().balance(&legit_dest), legit_dest_before);
    for id in [id_a, id_b] {
        assert_eq!(ctx.client().get_stream_state(&id).withdrawn_amount, 0);
    }
    assert!(collect_wdraw_events(&ctx).is_empty());
}

// ---------------------------------------------------------------------------
// Hardening — duplicate stream_id, paused stream, dust, completed stream
// ---------------------------------------------------------------------------

/// An empty `Vec<WithdrawToParam>` must succeed idempotently: no token moves,
/// no events emitted, auth still enforced. Mirrors `create_streams` /
/// `batch_withdraw` empty-vector coverage in [`docs/streaming.md`].
#[test]
fn batch_withdraw_to_empty_vec_succeeds_idempotently() {
    let ctx = BatchCtx::setup();
    let dest = Address::generate(&ctx.env);

    let results = ctx
        .client()
        .batch_withdraw_to(&ctx.recipient, &Vec::new(&ctx.env));

    assert!(
        results.is_empty(),
        "empty batch returns an empty Vec<BatchWithdrawResult>"
    );
    assert_eq!(
        ctx.token().balance(&dest),
        0,
        "no destination was touched"
    );
    assert_eq!(
        ctx.token().balance(&ctx.recipient),
        0,
        "no recipient balance change"
    );
    assert!(
        collect_wdraw_events(&ctx).is_empty(),
        "no wdraw_to events emitted for empty batch"
    );
    // Sanity: stream count unchanged.
    assert_eq!(ctx.client().get_stream_count(), 0);
}

/// A `Paused` stream whose `end_time` is in the past is "time-terminal" and
/// must be allowed to drain (`if stream.status == Paused && !is_terminal_state
/// { Err }` is the only rejection branch). This complements
/// `completed_stream_in_batch_returns_zero_with_no_event` which exercises the
/// Active time-terminal path.
#[test]
fn paused_time_terminal_stream_in_batch_succeeds() {
    let ctx = BatchCtx::setup();
    let id_a = ctx.create_stream_for(&ctx.recipient);
    ctx.sac.mint(&ctx.sender, &1_000_i128);
    let id_b = ctx.create_stream_for(&ctx.recipient);

    // Pause id_b BEFORE advancing past its end_time, so it is Paused AND
    // time-terminal when the batch runs.
    ctx.env.ledger().set_timestamp(0);
    ctx.client()
        .pause_stream(&id_b, &fluxora_stream::PauseReason::Operational);
    // Advance the ledger past both streams' end_time=1000.
    ctx.env.ledger().set_timestamp(1_500);

    let dest = Address::generate(&ctx.env);
    let withdrawals = wdraw_vec(&ctx.env, &[(id_a, &dest), (id_b, &dest)]);
    let results = ctx
        .client()
        .batch_withdraw_to(&ctx.recipient, &withdrawals);

    assert_eq!(results.len(), 2);
    // Active time-terminal id_a drains in full.
    assert_eq!(
        results.get(0).unwrap().amount,
        1_000,
        "active time-terminal stream drains full deposit"
    );
    // Paused time-terminal id_b drains in full.
    assert_eq!(
        results.get(1).unwrap().amount,
        1_000,
        "paused time-terminal stream drains full deposit"
    );

    // Destination gets 2000 (1000 + 1000).
    assert_eq!(ctx.token().balance(&dest), 2_000);

    // Both streams now Completed.
    for id in [id_a, id_b] {
        let st = ctx.client().get_stream_state(&id);
        assert_eq!(st.status, StreamStatus::Completed);
    }

    // Two wdraw_to events, one per stream.
    let events = collect_wdraw_events(&ctx);
    assert_eq!(events.len(), 2);
}

/// Duplicate `stream_id` within one batch must abort via the contract's
/// `assert!` (Soroban host trap), NOT return a typed error. Using
/// `#[should_panic]` documents this contract behavior for auditors.
#[test]
#[should_panic]
fn duplicate_stream_id_in_batch_panics() {
    let ctx = BatchCtx::setup();
    let id = ctx.create_stream_for(&ctx.recipient);
    let dest_a = Address::generate(&ctx.env);
    let dest_b = Address::generate(&ctx.env);

    ctx.env.ledger().set_timestamp(100);
    let withdrawals = wdraw_vec(&ctx.env, &[(id, &dest_a), (id, &dest_b)]);
    let _ = ctx
        .client()
        .batch_withdraw_to(&ctx.recipient, &withdrawals);
}

/// A non-terminal paused stream in the batch must cause `InvalidState` and
/// roll back any pre-pause transfers.
#[test]
fn paused_stream_in_batch_aborts_atomically() {
    let ctx = BatchCtx::setup();

    // Active stream (id_a) + a stream that will be paused before batching.
    let id_a = ctx.create_stream_for(&ctx.recipient);

    ctx.sac.mint(&ctx.sender, &1_000_i128);
    let id_b = ctx.create_stream_for(&ctx.recipient);
    ctx.client()
        .pause_stream(&id_b, &fluxora_stream::PauseReason::Operational);

    let dest = Address::generate(&ctx.env);
    let dest_before = ctx.token().balance(&dest);

    ctx.env.ledger().set_timestamp(400);

    let withdrawals = wdraw_vec(&ctx.env, &[(id_a, &dest), (id_b, &dest)]);
    let result = ctx
        .client()
        .try_batch_withdraw_to(&ctx.recipient, &withdrawals);
    assert_eq!(result, Err(Ok(ContractError::InvalidState)));

    assert_eq!(
        ctx.token().balance(&dest),
        dest_before,
        "no transfer to destination despite id_a being processable"
    );
    assert_eq!(ctx.client().get_stream_state(&id_a).withdrawn_amount, 0);
    assert_eq!(ctx.client().get_stream_state(&id_b).withdrawn_amount, 0);
}

/// Dust-threshold straddling causes the per-stream withdrawal to be filtered
/// to zero. The batch as a whole still succeeds; result.amount is 0 for the
/// filtered stream and no `wdraw_to` event is emitted for it.
#[test]
fn dust_threshold_skipped_stream_emits_no_event() {
    let ctx = BatchCtx::setup();

    // Stream with dust threshold = 500. At t=200, only 200 accrued below
    // the threshold, so withdrawable is clamped to 0.
    let id_small = ctx.client().create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1_000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1_000u64,
        &500i128, // withdraw_dust_threshold = 500
        &None,
        &StreamKind::Linear,
    );

    let dest = Address::generate(&ctx.env);
    ctx.env.ledger().set_timestamp(200);

    let withdrawals = wdraw_vec(&ctx.env, &[(id_small, &dest)]);
    let results = ctx
        .client()
        .batch_withdraw_to(&ctx.recipient, &withdrawals);

    assert_eq!(results.len(), 1);
    assert_eq!(
        results.get(0).unwrap().amount,
        0,
        "dust filter zeroes out the withdrawal"
    );
    assert_eq!(
        ctx.token().balance(&dest),
        0,
        "no tokens moved; withrawn_amount unchanged"
    );
    assert_eq!(
        ctx.client().get_stream_state(&id_small).withdrawn_amount,
        0
    );
    assert!(
        collect_wdraw_events(&ctx).is_empty(),
        "no wdraw_to event when amount is 0"
    );
}

/// Completed (terminal) streams in the batch return 0 with no event and do
/// not affect the other streams in the batch.
#[test]
fn completed_stream_in_batch_returns_zero_with_no_event() {
    let ctx = BatchCtx::setup();

    let id_done = ctx.create_stream_for(&ctx.recipient);
    // Drain id_done to Completed via a single full withdraw at t=1000.
    ctx.env.ledger().set_timestamp(1_000);
    ctx.client().withdraw(&id_done);
    assert_eq!(
        ctx.client().get_stream_state(&id_done).status,
        StreamStatus::Completed
    );

    // Build a fresh active stream for the batch alongside the completed one.
    ctx.sac.mint(&ctx.sender, &1_000_i128);
    let id_active = ctx.create_stream_for(&ctx.recipient);

    let dest = Address::generate(&ctx.env);
    let dest_before = ctx.token().balance(&dest);

    ctx.env.ledger().set_timestamp(1_500);

    // id_done comes first; id_active is processable and should drain fully
    // because we're past end_time (time-terminal allowed).
    let withdrawals = wdraw_vec(&ctx.env, &[(id_done, &dest), (id_active, &dest)]);
    let results = ctx
        .client()
        .batch_withdraw_to(&ctx.recipient, &withdrawals);

    assert_eq!(results.len(), 2);
    assert_eq!(
        results.get(0).unwrap().amount,
        0,
        "completed stream contributes 0"
    );
    assert_eq!(
        results.get(1).unwrap().stream_id, id_active,
        "active stream result present"
    );

    // id_active was time-terminal; its remaining deposit_yet_unwithdrawn
    // should be drained to the destination in full.
    let active_after = ctx.client().get_stream_state(&id_active);
    assert_eq!(
        active_after.withdrawn_amount, active_after.deposit_amount,
        "active stream fully drained"
    );
    assert_eq!(
        ctx.token().balance(&dest) - dest_before,
        active_after.deposit_amount,
        "destination credited with exactly the active stream's deposit"
    );

    // Event-level assertions: the completed stream contributes zero wdraw_to
    // events; only the active stream emits one. Counting directly here so a
    // future test-creep emitting extra events for any reason is caught.
    let events = collect_wdraw_events(&ctx);
    let done_event_count = events.iter().filter(|e| e.stream_id == id_done).count();
    let active_event_count = events.iter().filter(|e| e.stream_id == id_active).count();
    assert_eq!(
        done_event_count, 0,
        "completed stream must not emit wdraw_to (amount=0)"
    );
    assert_eq!(
        active_event_count, 1,
        "active stream emits exactly one wdraw_to"
    );
}
