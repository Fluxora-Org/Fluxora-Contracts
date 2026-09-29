//! Issue #1868 — the emitted event stream, on its own, must be enough for an
//! off-chain indexer to reconstruct every stream's state.
//!
//! # Why this module exists
//!
//! The contract keeps no per-party index — see the `lib.rs` module docs — so
//! "which streams are mine" can only be answered by indexing the events. That
//! makes the events load-bearing infrastructure, and the property they have to
//! hold is *reconstructability*: replaying the events alone, with no access to
//! `get_stream`, has to yield exactly the state the contract holds in storage.
//!
//! The existing event tests (`test::events`, `test::withdraw_events`,
//! `test::cancel_events`) each check one event kind against one operation. None
//! of them folds a whole history back into a `Stream`, which is the property
//! the issue actually asks for and the only one that catches a field that moved
//! in one event but is never republished in the next.
//!
//! # What is asserted here
//!
//! 1. [`reconstruction_from_events_alone_matches_storage`] replays a generated
//!    operation history — one that touches every state-changing entry point —
//!    through a fold that reads *only* event payloads, then compares **every**
//!    field of **every** reconstructed stream against `get_stream`.
//! 2. The same fold asserts, for each event, that it exposes the stream id and
//!    both parties of the stream *as it stands after that event*, so a consumer
//!    can route "streams that involve me" from any single event and not only
//!    from `stream_created`.
//! 3. The history records the event kind published by each call, which is the
//!    machine-checked form of "every state-changing entry point emits an
//!    event": a call that publishes nothing fails the step assertion, and a
//!    call that is never exercised fails the coverage assertion at the end.
//! 4. [`a_missing_field_breaks_reconstruction`] folds the same history with the
//!    pre-#1868 event schema — the one in which `withdrawn` and `cancelled` do
//!    not republish the pause bookkeeping — and requires the result to diverge,
//!    naming the fields involved. [`every_reconstructed_field_is_checked`]
//!    proves the comparator behind it is field-complete by mutating each field
//!    in turn.
//!
//! # The gap this closes
//!
//! `Stream` carries `paused_at` and `paused_total`, and two entry points move
//! them without a `paused`/`resumed` event of their own:
//!
//! * a `withdraw` that drains a *paused* stream to zero folds the in-progress
//!   pause into `paused_total` and clears `paused_at` (`lib.rs`,
//!   `apply_withdrawal`), and
//! * `cancel` clears `paused_at` on a stream that was paused at the time.
//!
//! Neither `withdrawn` nor `cancelled` used to publish either field, so an
//! event-only indexer kept a `paused_at` the contract had already dropped and a
//! `paused_total` it had already moved. Both events now republish the pair, and
//! the events that named only one side of the stream now name both; this module
//! is what holds them to it. Its measurements are also what justify the
//! event-byte budget re-derived in `test::resource_limits`.

#![cfg(test)]

use soroban_sdk::testutils::{Address as _, Events as _};
use soroban_sdk::{Address, Env, Map, Symbol, TryFromVal, TryIntoVal, Val};

use crate::op;
use crate::test::common::{Harness, DAY, ONE, T0};
use crate::{Stream, StreamStatus};

extern crate std;

// ---------------------------------------------------------------------------
// Event decoding
// ---------------------------------------------------------------------------

/// One contract event, split into the three pieces an indexer works with: the
/// `topic[0]` name symbol, the remaining topics, and the data payload decoded
/// as a `field name -> value` map.
struct Decoded {
    name: Symbol,
    /// Topics **including** `topic[0]`, matching what the contract published.
    topics: std::vec::Vec<Val>,
    data: Map<Symbol, Val>,
}

fn drain(h: &Harness<'_>) -> std::vec::Vec<soroban_sdk::xdr::ContractEvent> {
    h.env
        .events()
        .all()
        .filter_by_contract(&h.contract_id)
        .events()
        .to_vec()
}

fn decode(env: &Env, event: soroban_sdk::xdr::ContractEvent) -> Decoded {
    let soroban_sdk::xdr::ContractEventBody::V0(body) = event.body;
    let mut topics = std::vec::Vec::new();
    for t in body.topics.iter() {
        topics.push(Val::try_from_val(env, t).expect("a topic is a Val"));
    }
    let data_val: Val = Val::try_from_val(env, &body.data).expect("event data is a Val");
    let data: Map<Symbol, Val> = data_val
        .try_into_val(env)
        .expect("event data decodes as a Map<Symbol, Val>");
    let name: Symbol = topics
        .first()
        .expect("every contractevent publishes topic[0]")
        .try_into_val(env)
        .expect("topic[0] is the event name symbol");
    Decoded { name, topics, data }
}

fn key(env: &Env, name: &str) -> Symbol {
    Symbol::new(env, name)
}

/// Read `name` out of the payload, panicking if the event does not carry it.
///
/// The panic message is the failure mode acceptance criterion four asks for: a
/// field that is load-bearing for reconstruction but absent from the event
/// makes the reconstruction test fail, by name.
fn field<'a>(env: &'a Env, ev: &'a Decoded, name: &str) -> Val {
    ev.data.get(key(env, name)).unwrap_or_else(|| {
        panic!("event must publish `{name}` for the stream to be reconstructable from events alone")
    })
}

fn data_u64(env: &Env, ev: &Decoded, name: &str) -> u64 {
    field(env, ev, name).try_into_val(env).unwrap()
}

fn data_i128(env: &Env, ev: &Decoded, name: &str) -> i128 {
    field(env, ev, name).try_into_val(env).unwrap()
}

fn data_u32(env: &Env, ev: &Decoded, name: &str) -> u32 {
    field(env, ev, name).try_into_val(env).unwrap()
}

fn data_bool(env: &Env, ev: &Decoded, name: &str) -> bool {
    field(env, ev, name).try_into_val(env).unwrap()
}

fn data_addr(env: &Env, ev: &Decoded, name: &str) -> Address {
    field(env, ev, name).try_into_val(env).unwrap()
}

fn data_opt_u64(env: &Env, ev: &Decoded, name: &str) -> Option<u64> {
    field(env, ev, name).try_into_val(env).unwrap()
}

fn data_status(env: &Env, ev: &Decoded, name: &str) -> StreamStatus {
    field(env, ev, name).try_into_val(env).unwrap()
}

/// `topic[i]` decoded as a `u64`.
fn topic_u64(env: &Env, ev: &Decoded, i: u32) -> u64 {
    ev.topics
        .get(i as usize)
        .unwrap_or_else(|| panic!("event must publish a u64 at topic[{i}]"))
        .try_into_val(env)
        .expect("topic is a u64")
}

/// `topic[i]` decoded as an `Address`.
fn topic_addr(env: &Env, ev: &Decoded, i: u32) -> Address {
    ev.topics
        .get(i as usize)
        .unwrap_or_else(|| panic!("event must publish an Address at topic[{i}]"))
        .try_into_val(env)
        .expect("topic is an Address")
}

/// Every address an event exposes, in topics or payload.
///
/// Used for the routing assertion: whichever side of the stream an event names,
/// a consumer filtering by address has to be able to see it.
fn addresses_in(env: &Env, ev: &Decoded) -> std::vec::Vec<Address> {
    let mut found = std::vec::Vec::new();
    for v in ev.topics.iter() {
        if let Ok(a) = Address::try_from_val(env, v) {
            found.push(a);
        }
    }
    for (_k, v) in ev.data.iter() {
        if let Ok(a) = Address::try_from_val(env, &v) {
            found.push(a);
        }
    }
    found
}

// ---------------------------------------------------------------------------
// The reconstruction
// ---------------------------------------------------------------------------

/// A stream rebuilt from events alone. Mirrors every field of [`Stream`].
#[derive(Clone)]
struct Rebuilt {
    id: u64,
    sender: Address,
    recipient: Address,
    token: Address,
    deposited: i128,
    withdrawn: i128,
    start_time: u64,
    end_time: u64,
    cliff_time: u64,
    cancellable: bool,
    pausable: bool,
    transferable: bool,
    paused_at: Option<u64>,
    paused_total: u64,
    status: StreamStatus,
}

impl Rebuilt {
    /// Build the expected reconstruction straight from storage. Only used to
    /// seed the comparator test, never by the event fold.
    fn from_stream(id: u64, s: &Stream) -> Rebuilt {
        Rebuilt {
            id,
            sender: s.sender.clone(),
            recipient: s.recipient.clone(),
            token: s.token.clone(),
            deposited: s.deposited,
            withdrawn: s.withdrawn,
            start_time: s.start_time,
            end_time: s.end_time,
            cliff_time: s.cliff_time,
            cancellable: s.cancellable,
            pausable: s.pausable,
            transferable: s.transferable,
            paused_at: s.paused_at,
            paused_total: s.paused_total,
            status: s.status,
        }
    }
}

/// Field-by-field comparison. Returns the names of the fields that disagree, so
/// a failure says *which* field an indexer could not recover.
fn diffs(rebuilt: &Rebuilt, stored: &Stream) -> std::vec::Vec<&'static str> {
    let mut out = std::vec::Vec::new();
    if rebuilt.sender != stored.sender {
        out.push("sender");
    }
    if rebuilt.recipient != stored.recipient {
        out.push("recipient");
    }
    if rebuilt.token != stored.token {
        out.push("token");
    }
    if rebuilt.deposited != stored.deposited {
        out.push("deposited");
    }
    if rebuilt.withdrawn != stored.withdrawn {
        out.push("withdrawn");
    }
    if rebuilt.start_time != stored.start_time {
        out.push("start_time");
    }
    if rebuilt.end_time != stored.end_time {
        out.push("end_time");
    }
    if rebuilt.cliff_time != stored.cliff_time {
        out.push("cliff_time");
    }
    if rebuilt.cancellable != stored.cancellable {
        out.push("cancellable");
    }
    if rebuilt.pausable != stored.pausable {
        out.push("pausable");
    }
    if rebuilt.transferable != stored.transferable {
        out.push("transferable");
    }
    if rebuilt.paused_at != stored.paused_at {
        out.push("paused_at");
    }
    if rebuilt.paused_total != stored.paused_total {
        out.push("paused_total");
    }
    if rebuilt.status != stored.status {
        out.push("status");
    }
    out
}

/// Which event fields the fold is allowed to read.
///
/// The only user of a restricted schema is the negative test: folding with
/// [`Schema::PRE_1868`] simulates the event set as it was before the pause
/// bookkeeping was republished by `withdrawn`/`cancelled`.
#[derive(Clone, Copy)]
struct Schema {
    pause_fields_on_withdraw: bool,
    pause_fields_on_cancel: bool,
    check_parties: bool,
}

impl Schema {
    /// The deployed schema: everything the events carry is read.
    const CURRENT: Schema = Schema {
        pause_fields_on_withdraw: true,
        pause_fields_on_cancel: true,
        check_parties: true,
    };

    /// The schema before #1868: `withdrawn`/`cancelled` carry neither the
    /// pause bookkeeping nor the missing party.
    const PRE_1868: Schema = Schema {
        pause_fields_on_withdraw: false,
        pause_fields_on_cancel: false,
        check_parties: false,
    };
}

/// The event kinds that change the reconstructable state of a stream, and for
/// which the "stream id and both parties" guarantee is asserted.
fn is_stream_lifecycle(env: &Env, ev: &Decoded) -> bool {
    [
        "stream_created",
        "withdrawn",
        "cancelled",
        "paused",
        "resumed",
        "topped_up",
        "recipient_transferred",
    ]
    .iter()
    .any(|n| ev.name == key(env, n))
}

/// Replay `events` into one [`Rebuilt`] per stream, reading only event payloads.
///
/// Events for a stream that has not yet been created, and unknown event kinds,
/// are rejected rather than ignored — an indexer that silently skips an event is
/// exactly the failure this module guards against.
fn reconstruct(env: &Env, events: &[Decoded], schema: Schema) -> std::vec::Vec<Rebuilt> {
    let mut states: std::vec::Vec<Rebuilt> = std::vec::Vec::new();

    for ev in events {
        let name = |n: &str| ev.name == key(env, n);
        let id = topic_u64(env, ev, 1);

        if name("stream_created") {
            // `stream_id`, `sender` and `recipient` are the topics; the rest of
            // the initial state is the payload.
            states.push(Rebuilt {
                id,
                sender: topic_addr(env, ev, 2),
                recipient: topic_addr(env, ev, 3),
                token: data_addr(env, ev, "token"),
                deposited: data_i128(env, ev, "deposited"),
                withdrawn: 0,
                start_time: data_u64(env, ev, "start_time"),
                end_time: data_u64(env, ev, "end_time"),
                cliff_time: data_u64(env, ev, "cliff_time"),
                cancellable: data_bool(env, ev, "cancellable"),
                pausable: data_bool(env, ev, "pausable"),
                transferable: data_bool(env, ev, "transferable"),
                paused_at: None,
                paused_total: 0,
                status: StreamStatus::Active,
            });
        } else {
            let at = states.iter().position(|s| s.id == id).unwrap_or_else(|| {
                panic!("event for stream {id} arrived before its stream_created")
            });

            if name("withdrawn") {
                let s = &mut states[at];
                s.withdrawn = data_i128(env, ev, "withdrawn");
                s.deposited = data_i128(env, ev, "deposited");
                s.status = data_status(env, ev, "status");
                if schema.pause_fields_on_withdraw {
                    // A full withdrawal of a paused stream folds the
                    // in-progress pause into `paused_total` and clears
                    // `paused_at`; both are stream state and must be readable
                    // back off the event.
                    s.paused_at = data_opt_u64(env, ev, "paused_at");
                    s.paused_total = data_u64(env, ev, "paused_total");
                }
            } else if name("cancelled") {
                let s = &mut states[at];
                s.deposited = data_i128(env, ev, "vested");
                s.withdrawn = data_i128(env, ev, "withdrawn");
                s.end_time = data_u64(env, ev, "end_time");
                s.status = StreamStatus::Cancelled;
                if schema.pause_fields_on_cancel {
                    // Cancel closes any in-progress pause without a `resumed`
                    // event of its own, so the clearing has to be visible here.
                    s.paused_at = data_opt_u64(env, ev, "paused_at");
                    s.paused_total = data_u64(env, ev, "paused_total");
                }
            } else if name("paused") {
                let s = &mut states[at];
                s.paused_at = Some(data_u64(env, ev, "paused_at"));
                s.paused_total = data_u64(env, ev, "paused_total");
                s.status = StreamStatus::Paused;
            } else if name("resumed") {
                let s = &mut states[at];
                s.paused_total = data_u64(env, ev, "paused_total");
                s.paused_at = None;
                s.status = StreamStatus::Active;
            } else if name("topped_up") {
                let s = &mut states[at];
                s.deposited = data_i128(env, ev, "deposited");
                s.end_time = data_u64(env, ev, "end_time");
            } else if name("recipient_transferred") {
                states[at].recipient = topic_addr(env, ev, 3);
            } else if name("ttl_extended") {
                // Rent maintenance only: it moves no field of `Stream`. Assert
                // the payload is readable so the event is still attributable.
                let _ = data_u32(env, ev, "extended_to_ledgers");
            } else if name("delegate_granted") || name("delegate_revoked") {
                // Delegation metadata, not `Stream` state: reconstructing the
                // grants needs a second store, and they do not appear in
                // `get_stream`. The stream id is what ties them to a stream.
            } else {
                panic!("unknown event kind cannot be folded into stream state");
            }
        }

        // Routing guarantee: a stream-lifecycle event exposes the stream id and
        // both parties of the stream as they stand *after* the event, so a
        // consumer filtering by address sees every event that concerns it.
        if schema.check_parties && is_stream_lifecycle(env, ev) {
            let s = states.iter().find(|s| s.id == id).unwrap();
            for party in [&s.sender, &s.recipient] {
                assert!(
                    addresses_in(env, ev).contains(party),
                    "a stream-lifecycle event must expose both parties of the \
                     stream it concerns (stream {id})",
                );
            }
        }
    }

    states
}

// ---------------------------------------------------------------------------
// Recording the generated history
// ---------------------------------------------------------------------------

/// Runs a history, asserting the exact event kinds each call publishes.
struct Recorder {
    events: std::vec::Vec<Decoded>,
    entry_points: std::vec::Vec<&'static str>,
}

impl Recorder {
    fn new() -> Recorder {
        Recorder {
            events: std::vec::Vec::new(),
            entry_points: std::vec::Vec::new(),
        }
    }

    /// Drain the events published by the call that just ran and assert they are
    /// exactly `expected`, in order.
    ///
    /// `expected` is never empty: a state-changing entry point that publishes
    /// nothing fails here, which is the "every state-changing entry point emits
    /// an event" acceptance criterion, checked per call rather than in bulk.
    fn step(&mut self, h: &Harness<'_>, entry_point: &'static str, expected: &[&str]) {
        let decoded: std::vec::Vec<Decoded> =
            drain(h).into_iter().map(|e| decode(&h.env, e)).collect();
        let got: std::vec::Vec<Symbol> = decoded.iter().map(|d| d.name.clone()).collect();
        let want: std::vec::Vec<Symbol> = expected.iter().map(|n| Symbol::new(&h.env, n)).collect();
        assert_eq!(got, want, "`{entry_point}` published the wrong event kinds");
        assert!(
            !expected.is_empty(),
            "`{entry_point}` is state-changing but emits no event",
        );
        self.events.extend(decoded);
        self.entry_points.push(entry_point);
    }
}

/// Every entry point that changes state. A history that misses one fails
/// [`reconstruction_from_events_alone_matches_storage`].
const STATE_CHANGING_ENTRY_POINTS: &[&str] = &[
    "create_stream",
    "top_up",
    "withdraw",
    "batch_withdraw",
    "cancel",
    "pause",
    "resume",
    "transfer_recipient",
    "grant_delegate",
    "revoke_delegate",
    "delegate_withdraw",
    "delegate_cancel",
    "delegate_pause",
    "delegate_resume",
    "delegate_top_up",
    "delegate_transfer_recipient",
    "extend_stream_ttl",
    "batch_extend_ttl",
];

/// A generated operation history that drives every state-changing entry point,
/// including the two pause transitions that used to be invisible to an
/// event-only indexer.
fn generated_history(h: &Harness<'_>) -> Recorder {
    let mut rec = Recorder::new();

    // --- stream 0: the full lifecycle, with delegate-free operations ---------
    let s0 = h.create(
        1_000 * ONE,
        T0,
        T0 + 100 * DAY,
        T0 + 10 * DAY,
        true,
        true,
        true,
    );
    rec.step(h, "create_stream", &["stream_created"]);

    h.advance(20 * DAY);
    h.client.pause(&s0);
    rec.step(h, "pause", &["paused"]);

    h.advance(5 * DAY);
    h.client.withdraw(&s0, &Some(100 * ONE));
    rec.step(h, "withdraw", &["withdrawn"]);

    h.advance(5 * DAY);
    h.client.resume(&s0);
    rec.step(h, "resume", &["resumed"]);

    h.client.top_up(&s0, &(100 * ONE));
    rec.step(h, "top_up", &["topped_up"]);

    h.advance(DAY);
    h.client.transfer_recipient(&s0, &h.other);
    rec.step(h, "transfer_recipient", &["recipient_transferred"]);

    h.advance(DAY);
    h.client.withdraw(&s0, &None);
    rec.step(h, "withdraw", &["withdrawn"]);

    h.client.cancel(&s0);
    rec.step(h, "cancel", &["cancelled"]);

    h.client.extend_stream_ttl(&s0);
    rec.step(h, "extend_stream_ttl", &["ttl_extended"]);

    // --- stream 1: drained while paused -------------------------------------
    // Fully vested, then paused, then drained. Depletion folds the in-progress
    // pause into `paused_total` and clears `paused_at` with no `resumed` event.
    let now = h.now();
    let s1 = h.create(1_000 * ONE, now, now + 10 * DAY, now, false, true, false);
    rec.step(h, "create_stream", &["stream_created"]);

    h.warp_to(now + 10 * DAY);
    h.client.pause(&s1);
    rec.step(h, "pause", &["paused"]);

    h.advance(3 * DAY);
    h.client.withdraw(&s1, &None);
    rec.step(h, "withdraw", &["withdrawn"]);

    // --- stream 2: cancelled while paused -----------------------------------
    let now = h.now();
    let s2 = h.create(1_000 * ONE, now, now + 100 * DAY, now, true, true, true);
    rec.step(h, "create_stream", &["stream_created"]);

    h.advance(10 * DAY);
    h.client.pause(&s2);
    rec.step(h, "pause", &["paused"]);

    h.advance(5 * DAY);
    h.client.cancel(&s2);
    rec.step(h, "cancel", &["cancelled"]);

    // --- stream 3: every delegate entry point -------------------------------
    let now = h.now();
    let s3 = h.create(1_000 * ONE, now, now + 100 * DAY, now, true, true, true);
    rec.step(h, "create_stream", &["stream_created"]);

    // Sender-side ops and recipient-side ops are separate grants: a grant for
    // the same (stream, delegate) pair replaces the previous one wholesale, so
    // the two sides need two delegates.
    let sender_delegate = h.other.clone();
    let recipient_delegate = Address::generate(&h.env);

    h.client.grant_delegate(
        &s3,
        &h.sender,
        &sender_delegate,
        &(op::CANCEL | op::PAUSE | op::RESUME | op::TOP_UP),
        &None,
    );
    rec.step(h, "grant_delegate", &["delegate_granted"]);

    h.client.grant_delegate(
        &s3,
        &h.recipient,
        &recipient_delegate,
        &(op::WITHDRAW | op::TRANSFER_RECIPIENT),
        &None,
    );
    rec.step(h, "grant_delegate", &["delegate_granted"]);

    h.client.delegate_pause(&s3, &sender_delegate);
    rec.step(h, "delegate_pause", &["paused"]);

    h.advance(2 * DAY);
    h.client.delegate_resume(&s3, &sender_delegate);
    rec.step(h, "delegate_resume", &["resumed"]);

    h.client.delegate_top_up(&s3, &sender_delegate, &(50 * ONE));
    rec.step(h, "delegate_top_up", &["topped_up"]);

    h.advance(DAY);
    h.client
        .delegate_withdraw(&s3, &recipient_delegate, &Some(10 * ONE));
    rec.step(h, "delegate_withdraw", &["withdrawn"]);

    let new_recipient = Address::generate(&h.env);
    h.client
        .delegate_transfer_recipient(&s3, &recipient_delegate, &new_recipient);
    rec.step(h, "delegate_transfer_recipient", &["recipient_transferred"]);

    h.advance(DAY);
    h.client.delegate_cancel(&s3, &sender_delegate);
    rec.step(h, "delegate_cancel", &["cancelled"]);

    h.client.revoke_delegate(&s3, &h.sender, &sender_delegate);
    rec.step(h, "revoke_delegate", &["delegate_revoked"]);

    // --- streams 4 and 5: the batch entry points ----------------------------
    let now = h.now();
    let s4 = h.create(1_000 * ONE, now, now + 100 * DAY, now, true, true, true);
    rec.step(h, "create_stream", &["stream_created"]);
    let s5 = h.create(1_000 * ONE, now, now + 100 * DAY, now, true, true, true);
    rec.step(h, "create_stream", &["stream_created"]);

    h.advance(10 * DAY);
    h.client.batch_withdraw(&h.recipient, &h.ids(&[s4, s5]));
    rec.step(h, "batch_withdraw", &["withdrawn", "withdrawn"]);

    h.client.batch_extend_ttl(&h.ids(&[s4, s5]));
    rec.step(h, "batch_extend_ttl", &["ttl_extended", "ttl_extended"]);

    rec
}

// ---------------------------------------------------------------------------
// 1 + 2 + 3 — replay the history from events alone and compare to storage
// ---------------------------------------------------------------------------

#[test]
fn reconstruction_from_events_alone_matches_storage() {
    let h = Harness::new();
    let rec = generated_history(&h);

    // Every state-changing entry point is exercised (acceptance criterion two).
    let mut covered = rec.entry_points.clone();
    covered.sort();
    covered.dedup();
    let mut expected = STATE_CHANGING_ENTRY_POINTS.to_vec();
    expected.sort();
    assert_eq!(
        covered, expected,
        "the generated history must drive every state-changing entry point",
    );

    let rebuilt = reconstruct(&h.env, &rec.events, Schema::CURRENT);

    let count = h.client.stream_count();
    assert_eq!(
        rebuilt.len() as u64,
        count,
        "every stream the contract knows about must be discoverable from events alone",
    );

    for state in &rebuilt {
        let stored: Stream = h.client.get_stream(&state.id);
        let mismatched = diffs(state, &stored);
        assert!(
            mismatched.is_empty(),
            "stream {} is not reconstructable from its events; fields an indexer \
             cannot recover: {mismatched:?}\n\
             rebuilt: deposited={} withdrawn={} end_time={} paused_at={:?} \
             paused_total={} status={:?}\n\
             stored:  deposited={} withdrawn={} end_time={} paused_at={:?} \
             paused_total={} status={:?}",
            state.id,
            state.deposited,
            state.withdrawn,
            state.end_time,
            state.paused_at,
            state.paused_total,
            state.status,
            stored.deposited,
            stored.withdrawn,
            stored.end_time,
            stored.paused_at,
            stored.paused_total,
            stored.status,
        );
    }
}

// ---------------------------------------------------------------------------
// 4 — a missing field breaks reconstruction, and the comparator notices
// ---------------------------------------------------------------------------

/// The same history, folded with the pre-#1868 schema, must diverge: this is
/// the regression guard that makes the pause bookkeeping on `withdrawn` and
/// `cancelled` load-bearing rather than decorative.
#[test]
fn a_missing_field_breaks_reconstruction() {
    let h = Harness::new();
    let rec = generated_history(&h);

    let legacy = reconstruct(&h.env, &rec.events, Schema::PRE_1868);

    let mut diverged: std::vec::Vec<(u64, std::vec::Vec<&'static str>)> = std::vec::Vec::new();
    for state in &legacy {
        let stored: Stream = h.client.get_stream(&state.id);
        let mismatched = diffs(state, &stored);
        if !mismatched.is_empty() {
            diverged.push((state.id, mismatched));
        }
    }

    assert!(
        !diverged.is_empty(),
        "the pre-#1868 schema must NOT be able to reconstruct these streams — \
         if it can, the pause bookkeeping republished by withdrawn/cancelled is \
         untested",
    );

    // The two pause transitions that moved state without an event of their own
    // are exactly the ones that used to be unrecoverable, on both fields.
    for (id, fields) in &diverged {
        std::println!("pre-#1868 fold diverged on stream {id}: {fields:?}");
    }
    assert!(
        diverged
            .iter()
            .any(|(_, f)| f.contains(&"paused_at") && f.contains(&"paused_total")),
        "the divergence must be the pause bookkeeping moved by withdrawn/cancelled",
    );
}

/// The comparator behind the test above must look at every field of `Stream`.
/// Mutate each one in turn and require it to be named.
#[test]
fn every_reconstructed_field_is_checked() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let stored = h.get(id);

    let base = Rebuilt::from_stream(id, &stored);
    assert!(
        diffs(&base, &stored).is_empty(),
        "the seeded reconstruction must match storage",
    );

    let check = |mutated: Rebuilt, field: &str| {
        assert_eq!(
            diffs(&mutated, &stored),
            std::vec![field],
            "mutating `{field}` must be detected",
        );
    };

    let mut m = base.clone();
    m.sender = Address::generate(&h.env);
    check(m, "sender");

    let mut m = base.clone();
    m.recipient = Address::generate(&h.env);
    check(m, "recipient");

    let mut m = base.clone();
    m.token = Address::generate(&h.env);
    check(m, "token");

    let mut m = base.clone();
    m.deposited += 1;
    check(m, "deposited");

    let mut m = base.clone();
    m.withdrawn += 1;
    check(m, "withdrawn");

    let mut m = base.clone();
    m.start_time += 1;
    check(m, "start_time");

    let mut m = base.clone();
    m.end_time += 1;
    check(m, "end_time");

    let mut m = base.clone();
    m.cliff_time += 1;
    check(m, "cliff_time");

    let mut m = base.clone();
    m.cancellable = !m.cancellable;
    check(m, "cancellable");

    let mut m = base.clone();
    m.pausable = !m.pausable;
    check(m, "pausable");

    let mut m = base.clone();
    m.transferable = !m.transferable;
    check(m, "transferable");

    let mut m = base.clone();
    m.paused_at = Some(T0);
    check(m, "paused_at");

    let mut m = base.clone();
    m.paused_total += 1;
    check(m, "paused_total");

    let mut m = base.clone();
    m.status = StreamStatus::Cancelled;
    check(m, "status");
}
