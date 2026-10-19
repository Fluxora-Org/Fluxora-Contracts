//! Issue #1852 — TTL extension on a stream at the minimum TTL floor.
//!
//! `MIN_STREAM_TTL_LEDGERS` (30 days at the nominal 5 seconds per ledger,
//! 518,400 ledgers) is the rent a stream entry gets once its schedule has
//! fully elapsed: `storage::ttl_target_ledgers` computes
//! `ceil((remaining_life + TTL_BUFFER_SECONDS) / 5)` and floors it at the
//! minimum. The floor is where an extension either takes effect or silently
//! does nothing, and neither outcome was covered before this module.
//!
//! The tests here pin, through the public ABI only:
//!
//! 1. a settled stream targets exactly the floor;
//! 2. the floor starts to bind exactly at `end_time` (one ledger earlier the
//!    target is one ledger above it);
//! 3. an extension that only has to clear the floor still writes;
//! 4. extending an entry already at the floor is a safe no-op;
//! 5. an entry holding more rent than the floor is never clipped down to it;
//! 6. a floor extension preserves accounting and conserves funds end to end;
//! 7. a terminal `Depleted` stream still gets its floor;
//! 8. a keeper sweep over already-floored entries is idempotent.
//!
//! Every claim asserts both sides of its boundary, so an off-by-one cannot
//! pass. Funds conservation is asserted with the harness' `assert_pool_exact`.
//!
//! # What the host lets us observe
//!
//! `Harness::ttl_of` reads the entry's remaining TTL from the contract's
//! storage, and `age_ledgers` moves only the ledger sequence, leaving the
//! clock alone — enough to decay an entry below its target without disturbing
//! the timestamp-driven target computation. Reading an *expired* entry would
//! make the host auto-restore it (see `test::ttl`), so no test here ever
//! ages an entry that far.

use soroban_sdk::testutils::{Events as _, Ledger as _};
use soroban_sdk::{Symbol, TryFromVal, TryIntoVal, Val};

use super::common::*;
use crate::{storage, StreamStatus};

/// The floor every settled stream's extension targets.
const MIN: u32 = storage::MIN_STREAM_TTL_LEDGERS;

/// A 100-day schedule. At 5 seconds per ledger its creation TTL is
/// `seconds_to_ledgers(100 days + TTL_BUFFER_SECONDS)` = 2,246,400 ledgers,
/// comfortably below the host's `max_entry_ttl`.
const DURATION: u64 = 100 * DAY;

/// Deposit for the fixtures. The 100-day schedule clears the dust-rate gate.
const DEPOSIT: i128 = 1_000 * ONE;

/// Advance only the ledger sequence, leaving the clock alone.
///
/// Used to decay an entry's rent below its (timestamp-derived) target without
/// moving the schedule. Mirrors the helper in `test::ttl`.
fn age_ledgers(h: &Harness, ledgers: u32) {
    let seq = h.env.ledger().sequence();
    h.env.ledger().set_sequence_number(seq + ledgers);
}

/// All `ttl_extended` events from the most recent contract invocation, as
/// `(stream_id, extended_to_ledgers)` pairs in emission order.
///
/// Reads `topic[0]` (the event name), `topic[1]` (the stream id) and the
/// `extended_to_ledgers` payload key directly, so it asserts the wire shape
/// `docs/ABI.md` documents rather than a Rust-side re-encoding.
fn ttl_extended_events(h: &Harness) -> std::vec::Vec<(u64, u32)> {
    let raw = h
        .env
        .events()
        .all()
        .filter_by_contract(&h.contract_id)
        .events()
        .to_vec();

    let mut out = std::vec::Vec::new();
    for event in raw {
        let soroban_sdk::xdr::ContractEventBody::V0(body) = event.body;
        let mut topics = soroban_sdk::vec![&h.env];
        for t in body.topics.iter() {
            topics.push_back(Val::try_from_val(&h.env, t).expect("topic must decode"));
        }

        let name: Symbol = topics
            .get(0)
            .expect("every contract event has topic[0]")
            .try_into_val(&h.env)
            .expect("topic[0] must be the event name Symbol");
        if name != Symbol::new(&h.env, "ttl_extended") {
            continue;
        }

        let stream_id: u64 = topics
            .get(1)
            .expect("ttl_extended carries stream_id as topic[1]")
            .try_into_val(&h.env)
            .expect("topic[1] must be a u64");
        let data: Val = Val::try_from_val(&h.env, &body.data).expect("event data must decode");
        let payload: soroban_sdk::Map<Symbol, Val> = data
            .try_into_val(&h.env)
            .expect("ttl_extended payload must be a map");
        let extended: u32 = payload
            .get(Symbol::new(&h.env, "extended_to_ledgers"))
            .expect("payload must carry extended_to_ledgers")
            .try_into_val(&h.env)
            .expect("extended_to_ledgers must be a u32");

        out.push((stream_id, extended));
    }
    out
}

/// `seconds_to_ledgers(DURATION + TTL_BUFFER_SECONDS)` — the rent a freshly
/// created 100-day stream is funded for.
fn creation_ttl() -> u32 {
    storage::seconds_to_ledgers(DURATION + storage::TTL_BUFFER_SECONDS)
}

// ---------------------------------------------------------------------------
// 1. A settled stream targets exactly the floor
// ---------------------------------------------------------------------------

/// Once the schedule has elapsed, `extend_stream_ttl` funds the entry for
/// exactly `MIN_STREAM_TTL_LEDGERS` — no more. The entry is first allowed to
/// decay below the floor so the extension genuinely has work to do, and the
/// returned value, the stored TTL and the emitted event must all agree.
#[test]
fn a_settled_stream_targets_exactly_the_floor() {
    let h = Harness::new();
    let id = h.create_simple(DEPOSIT, DURATION);
    assert_eq!(h.ttl_of(id), creation_ttl());

    // Settle the schedule, and let the creation rent decay below the floor.
    h.warp_to(T0 + DURATION + 10 * DAY);
    let before = h.ttl_of(id);
    assert!(
        before < MIN,
        "precondition: a settled entry must sit below the floor, got {before}"
    );
    assert_eq!(
        h.client.vested_of(&id),
        DEPOSIT,
        "schedule must be fully elapsed"
    );

    let funded = h.client.extend_stream_ttl(&id);
    // Capture events before any `Harness::ttl_of` call: `as_contract` clears
    // the host's event buffer.
    let events = ttl_extended_events(&h);
    assert_eq!(funded, MIN, "a settled stream targets exactly the floor");
    assert_eq!(
        h.ttl_of(id),
        MIN,
        "stored TTL must match the returned target"
    );
    assert!(h.ttl_of(id) > before, "the extension must take effect");

    // The emitted event carries the same number the ABI promises.
    assert_eq!(events, std::vec![(id, MIN)]);

    // Nothing about the stream or its backing moved.
    let s = h.get(id);
    assert_eq!(s.status, StreamStatus::Active);
    assert_eq!(s.deposited, DEPOSIT);
    assert_eq!(s.withdrawn, 0);
    assert_eq!(h.pool(), DEPOSIT);
    h.assert_pool_exact();
}

// ---------------------------------------------------------------------------
// 2. The floor starts to bind exactly at the stream's end
// ---------------------------------------------------------------------------

/// The target is `MIN + 1` with exactly one ledger (5 seconds) of life left,
/// and exactly `MIN` at `end_time`. Both sides of the boundary are asserted,
/// so an off-by-one in the ledger conversion cannot pass.
#[test]
fn the_floor_starts_to_bind_exactly_at_the_streams_end() {
    let h = Harness::new();
    let id = h.create_simple(DEPOSIT, DURATION);

    // --- one ledger (5 seconds) before the end: one above the floor ---
    h.advance(DURATION - 5);
    assert_eq!(h.now(), T0 + DURATION - 5);
    // Decay below the target so the extension has to write.
    age_ledgers(&h, 1_000);
    let before = h.ttl_of(id);
    let funded = h.client.extend_stream_ttl(&id);
    assert_eq!(
        funded,
        MIN + 1,
        "one ledger of remaining life is worth exactly one ledger above the floor"
    );
    assert_eq!(h.ttl_of(id), MIN + 1);
    assert!(funded > MIN, "the floor must not bind one ledger early");
    assert!(h.ttl_of(id) > before);

    // --- exactly at the end: the floor binds ---
    h.advance(5);
    assert_eq!(h.now(), T0 + DURATION);
    age_ledgers(&h, 1_000);
    let before = h.ttl_of(id);
    let funded = h.client.extend_stream_ttl(&id);
    assert_eq!(funded, MIN, "at the end the target is exactly the floor");
    assert_eq!(h.ttl_of(id), MIN);
    assert!(h.ttl_of(id) > before, "the extension must take effect");

    h.assert_pool_exact();
}

// ---------------------------------------------------------------------------
// 3. An extension that only has to clear the floor still takes effect
// ---------------------------------------------------------------------------

/// One ledger past the end, the entry has decayed to `MIN - 1`. Extending has
/// exactly one ledger of work to do, and must do it: the stored TTL rises to
/// the floor, the returned value and event payload are the floor, and the
/// extension does not move the ledger clock.
#[test]
fn an_extension_that_only_has_to_clear_the_floor_still_takes_effect() {
    let h = Harness::new();
    let id = h.create_simple(DEPOSIT, DURATION);

    h.advance(DURATION);
    assert_eq!(h.now(), T0 + DURATION);
    assert_eq!(h.ttl_of(id), MIN, "the settled entry lands on the floor");

    // Decay one ledger so the entry is strictly below the floor.
    age_ledgers(&h, 1);
    let before = h.ttl_of(id);
    assert_eq!(before, MIN - 1);
    let seq_before = h.env.ledger().sequence();

    let funded = h.client.extend_stream_ttl(&id);
    let events = ttl_extended_events(&h);
    assert_eq!(
        h.env.ledger().sequence(),
        seq_before,
        "the clock must not move"
    );
    assert_eq!(funded, MIN, "the target is the bare floor");
    assert_eq!(h.ttl_of(id), MIN, "the extension cleared the floor");
    assert_eq!(
        h.ttl_of(id) - before,
        1,
        "exactly the one-ledger deficit must be restored"
    );
    assert_eq!(events, std::vec![(id, MIN)]);

    h.assert_pool_exact();
}

// ---------------------------------------------------------------------------
// 4. Extending an entry already at the floor is a safe no-op
// ---------------------------------------------------------------------------

/// A second extension of an entry already sitting at the floor must not write
/// anything: the same ledger entries come back bit-for-bit, the returned
/// target is still the floor, and the nonce stays put. This is what makes a
/// keeper safe to run on a schedule without a watermark.
#[test]
fn extending_an_entry_already_at_the_floor_is_a_safe_no_op() {
    let h = Harness::new();
    let id = h.create_simple(DEPOSIT, DURATION);
    h.warp_to(T0 + DURATION + 10 * DAY);

    let first = h.client.extend_stream_ttl(&id);
    assert_eq!(first, MIN);
    assert_eq!(h.ttl_of(id), MIN);

    let stream_before = h.get(id);
    let seq_before = h.env.ledger().sequence();
    let entries_before = h.env.to_ledger_snapshot().ledger_entries;

    let second = h.client.extend_stream_ttl(&id);

    let entries_after = h.env.to_ledger_snapshot().ledger_entries;
    assert_eq!(second, MIN, "the target is unchanged");
    assert_eq!(h.ttl_of(id), MIN, "the TTL is unchanged");
    assert_eq!(h.env.ledger().sequence(), seq_before);
    assert_eq!(h.get(id), stream_before, "stream data must be untouched");
    assert!(
        entries_before == entries_after,
        "extending an entry already at the floor wrote storage"
    );
    h.assert_pool_exact();
}

// ---------------------------------------------------------------------------
// 5. A settled entry above the floor is not clipped down to it
// ---------------------------------------------------------------------------

/// `extend_ttl` only ever moves an entry's `live_until` forward. Here the
/// clock is jumped past the end while the ledger sequence is left in place, so
/// the entry still holds its full creation rent while the target has collapsed
/// to the floor. The extension must leave the entry alone rather than clip it
/// down.
#[test]
fn a_settled_entry_above_the_floor_is_not_clipped_down_to_it() {
    let h = Harness::new();
    let id = h.create_simple(DEPOSIT, DURATION);
    let created = h.ttl_of(id);
    assert_eq!(created, creation_ttl());
    assert!(created > MIN, "the fixture must start above the floor");

    // Settle the schedule by moving only the clock: the entry keeps its
    // creation rent while the target becomes the floor.
    h.env.ledger().set_timestamp(T0 + DURATION + 10 * DAY);
    let before = h.ttl_of(id);
    assert_eq!(
        before, created,
        "moving only the clock must not decay the entry"
    );
    assert_eq!(h.client.vested_of(&id), DEPOSIT);

    let funded = h.client.extend_stream_ttl(&id);
    assert_eq!(funded, MIN, "the computed target is the floor");
    assert_eq!(
        h.ttl_of(id),
        before,
        "an extension must never shorten an entry"
    );
    assert!(
        h.ttl_of(id) > MIN,
        "the entry must not be clipped down to the floor"
    );

    let s = h.get(id);
    assert_eq!(s.deposited, DEPOSIT);
    assert_eq!(s.withdrawn, 0);
    h.assert_pool_exact();
}

// ---------------------------------------------------------------------------
// 6. A floor extension preserves accounting and conserves funds
// ---------------------------------------------------------------------------

/// End to end: settle a stream, extend it at the floor, then withdraw
/// everything. The extension moves no tokens, the payout is exact, the stream
/// ends `Depleted`, and sender + recipient account for every minted unit.
#[test]
fn a_floor_extension_preserves_accounting_and_conserves_funds() {
    let h = Harness::new();
    let minted = 1_000_000 * ONE;
    let id = h.create_simple(DEPOSIT, DURATION);
    assert_eq!(h.balance(&h.sender), minted - DEPOSIT);

    h.warp_to(T0 + DURATION + 10 * DAY);
    let pool_before = h.pool();
    assert_eq!(pool_before, DEPOSIT);

    let funded = h.client.extend_stream_ttl(&id);
    assert_eq!(funded, MIN);
    assert_eq!(h.ttl_of(id), MIN);
    assert_eq!(h.pool(), pool_before, "a TTL extension must not move funds");

    let paid = h.client.withdraw(&id, &None);
    assert_eq!(paid, DEPOSIT, "the full deposit must pay out");

    let s = h.get(id);
    assert_eq!(s.deposited, DEPOSIT);
    assert_eq!(s.withdrawn, DEPOSIT);
    assert_eq!(s.status, StreamStatus::Depleted);
    assert_eq!(h.balance(&h.recipient), DEPOSIT);
    assert_eq!(h.pool(), 0, "nothing may be left stranded");
    assert_eq!(
        h.balance(&h.sender) + h.balance(&h.recipient),
        minted,
        "sender + recipient must account for every minted unit"
    );
    h.assert_pool_exact();
}

// ---------------------------------------------------------------------------
// 7. A depleted stream still gets its floor
// ---------------------------------------------------------------------------

/// `extend_stream_ttl` is documented as valid for terminal streams. After a
/// full withdrawal the entry is `Depleted`; its rent must still be restorable
/// to the floor, with the terminal state and the (now empty) accounting
/// untouched.
#[test]
fn a_depleted_stream_still_gets_its_floor() {
    let h = Harness::new();
    let id = h.create_simple(DEPOSIT, DURATION);
    h.warp_to(T0 + DURATION + 10 * DAY);
    assert_eq!(h.client.withdraw(&id, &None), DEPOSIT);

    let depleted = h.get(id);
    assert_eq!(depleted.status, StreamStatus::Depleted);
    assert_eq!(h.pool(), 0);

    // Let the terminal entry's rent decay below the floor.
    age_ledgers(&h, MIN + 1);
    let before = h.ttl_of(id);
    assert!(before < MIN, "precondition: entry decayed below the floor");

    let funded = h.client.extend_stream_ttl(&id);
    let events = ttl_extended_events(&h);
    assert_eq!(funded, MIN);
    assert_eq!(h.ttl_of(id), MIN);
    assert_eq!(events, std::vec![(id, MIN)]);

    assert_eq!(h.get(id), depleted, "terminal state must be untouched");
    assert_eq!(h.pool(), 0);
    h.assert_pool_exact();
}

// ---------------------------------------------------------------------------
// 8. A keeper sweep over floored entries is idempotent
// ---------------------------------------------------------------------------

/// Three settled streams, all decayed below the floor, swept with
/// `batch_extend_ttl`. The first sweep funds each to exactly the floor and
/// emits one event per stream; a second sweep writes nothing at all — the
/// ledger snapshot is identical — while still reporting three entries and the
/// floor target.
#[test]
fn a_keeper_sweep_over_floored_entries_is_idempotent() {
    let h = Harness::new();
    let a = h.create_simple(DEPOSIT, 100 * DAY);
    let b = h.create_simple(DEPOSIT, 110 * DAY);
    let c = h.create_simple(DEPOSIT, 120 * DAY);

    // 125 days: every schedule has fully elapsed, and every entry is still
    // live (the shortest creation TTL covers 130 days) but below the floor.
    h.warp_to(T0 + 125 * DAY);
    for id in [a, b, c] {
        assert!(
            h.ttl_of(id) < MIN,
            "precondition: stream {id} must sit below the floor"
        );
        assert!(
            h.ttl_of(id) > 0,
            "precondition: stream {id} must not archive"
        );
    }

    // First sweep: each entry is funded to exactly the floor. Events are
    // captured before any `ttl_of` call, which clears the host event buffer.
    let first = h.client.batch_extend_ttl(&h.ids(&[a, b, c]));
    let first_events = ttl_extended_events(&h);
    assert_eq!(first, 3);
    assert_eq!(
        first_events,
        std::vec![(a, MIN), (b, MIN), (c, MIN)],
        "one floor event per swept stream, in id order"
    );
    for id in [a, b, c] {
        assert_eq!(h.ttl_of(id), MIN);
    }

    // Second sweep: already floored, so nothing changes.
    let streams_before = [h.get(a), h.get(b), h.get(c)];
    let entries_before = h.env.to_ledger_snapshot().ledger_entries;

    let second = h.client.batch_extend_ttl(&h.ids(&[a, b, c]));
    let second_events = ttl_extended_events(&h);
    let entries_after = h.env.to_ledger_snapshot().ledger_entries;

    assert_eq!(second, 3, "the sweep still reports every existing stream");
    assert_eq!(second_events, std::vec![(a, MIN), (b, MIN), (c, MIN)]);
    for (i, id) in [a, b, c].into_iter().enumerate() {
        assert_eq!(h.ttl_of(id), MIN, "the floor is stable across sweeps");
        assert_eq!(h.get(id), streams_before[i], "stream {id} data changed");
    }
    assert!(
        entries_before == entries_after,
        "the second sweep over already-floored entries wrote storage"
    );
    h.assert_pool_exact();
}
