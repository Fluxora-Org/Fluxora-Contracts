//! Stage 3 — resource consumption.
//!
//! Any design that iterates over a growing collection works fine with five
//! streams and fails in production with five hundred, so the batch cap has to
//! be justified by measurement rather than by a guess. These tests print what
//! they measure and assert headroom against it.
//!
//! # The limits that actually bind, on protocol 27
//!
//! The familiar "roughly 200 ledger entry reads" figure predates protocol 23.
//! Since then live Soroban state is held in memory, and the read limit that
//! survives (`disk_read_entries`, still 200) counts only entries restored from
//! disk plus non-Soroban entries such as classic account balances — for a
//! contract touching live state it is usually zero.
//!
//! What binds instead, from the mainnet snapshot the SDK enforces:
//!
//! | limit | value |
//! |---|---|
//! | `ledger_entries` (total footprint: disk reads + memory reads + writes) | 400 |
//! | `write_entries` | 200 |
//! | `instructions` | 400,000,000 |
//! | `contract_events_size_bytes` | 16,384 |
//!
//! The footprint total is the real ceiling on batch size, and the event budget
//! is the one people forget — a batch emitting one event per stream can run out
//! of event bytes before it runs out of entries.
//!
//! # Caveats on the numbers
//!
//! * These contracts are registered natively, so Wasm instantiation and
//!   execution costs are skipped and measured *instructions* understate a real
//!   deployment. Entry counts, which is what the cap is about, are accurate.
//! * The mainnet limits are enforced by default on every invocation here, so
//!   anything that would be rejected on-network fails the test outright rather
//!   than merely reporting a large number.
//! * The limits are a snapshot taken when the SDK was published, not a live
//!   query. An SDK upgrade can move them; that is one more reason this file
//!   exists.

use super::common::*;
use crate::{Error, MAX_BATCH_SIZE};

/// Maximum total transaction footprint: disk reads + memory reads + writes.
const LEDGER_ENTRY_LIMIT: u32 = 400;
/// Maximum entries one transaction may write.
const WRITE_ENTRY_LIMIT: u32 = 200;
/// Maximum total size of emitted contract events, in bytes.
const EVENT_BYTES_LIMIT: u32 = 16_384;

#[derive(Debug, Clone, Copy)]
struct Cost {
    footprint: u32,
    writes: u32,
    instructions: i64,
    event_bytes: u32,
}

/// Report and return the last invocation's cost.
fn report(h: &Harness, label: &str) -> Cost {
    let r = h.env.cost_estimate().resources();
    let cost = Cost {
        footprint: r.disk_read_entries + r.memory_read_entries + r.write_entries,
        writes: r.write_entries,
        instructions: r.instructions,
        event_bytes: r.contract_events_size_bytes,
    };
    std::println!(
        "{label:<26} footprint={:<4}/{LEDGER_ENTRY_LIMIT}  writes={:<4}/{WRITE_ENTRY_LIMIT}  \
         events={:<6}/{EVENT_BYTES_LIMIT}  instructions={}",
        cost.footprint,
        cost.writes,
        cost.event_bytes,
        cost.instructions,
    );
    cost
}

fn assert_has_headroom(label: &str, cost: Cost, factor: u32) {
    assert!(
        cost.footprint * factor <= LEDGER_ENTRY_LIMIT,
        "{label}: footprint {} lacks {factor}x headroom under {LEDGER_ENTRY_LIMIT}",
        cost.footprint,
    );
    assert!(
        cost.writes * factor <= WRITE_ENTRY_LIMIT,
        "{label}: {} writes lack {factor}x headroom under {WRITE_ENTRY_LIMIT}",
        cost.writes,
    );
    assert!(
        cost.event_bytes * factor <= EVENT_BYTES_LIMIT,
        "{label}: {} event bytes lack {factor}x headroom under {EVENT_BYTES_LIMIT}",
        cost.event_bytes,
    );
}

/// A single-stream withdraw touches the instance entry, one stream entry and
/// the token's entries. It must be nowhere near any ceiling.
#[test]
fn a_single_withdraw_is_cheap() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(30 * DAY);

    h.client.withdraw(&id, &None);
    let cost = report(&h, "withdraw");

    assert!(
        cost.footprint < 20,
        "footprint {} is larger than expected",
        cost.footprint
    );
    assert_has_headroom("withdraw", cost, 10);
}

#[test]
fn every_single_stream_operation_has_wide_headroom() {
    let h = Harness::new();

    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    assert_has_headroom("create_stream", report(&h, "create_stream"), 10);

    h.advance(10 * DAY);
    h.client.top_up(&id, &(100 * ONE));
    assert_has_headroom("top_up", report(&h, "top_up"), 10);

    h.client.pause(&id);
    assert_has_headroom("pause", report(&h, "pause"), 10);

    h.client.resume(&id);
    assert_has_headroom("resume", report(&h, "resume"), 10);

    h.client.withdraw(&id, &None);
    assert_has_headroom("withdraw", report(&h, "withdraw"), 10);

    h.client.transfer_recipient(&id, &h.other);
    assert_has_headroom("transfer_recipient", report(&h, "transfer_recipient"), 10);

    h.client.extend_stream_ttl(&id);
    assert_has_headroom("extend_stream_ttl", report(&h, "extend_stream_ttl"), 10);

    h.client.cancel(&id);
    assert_has_headroom("cancel", report(&h, "cancel"), 10);
}

/// **The batch cap justification.** At exactly [`MAX_BATCH_SIZE`] the
/// transaction must still have room for protocol drift and for a heavier token
/// contract than the SAC used here — so the bar is 2x headroom, not merely
/// "fits".
#[test]
fn a_full_batch_withdraw_keeps_headroom_on_every_limit() {
    let h = Harness::new();
    let ids: std::vec::Vec<u64> = (0..MAX_BATCH_SIZE)
        .map(|_| h.create_simple(100 * ONE, 100 * DAY))
        .collect();
    h.advance(30 * DAY);

    h.client.batch_withdraw(&h.recipient, &h.ids(&ids));
    let cost = report(&h, "batch_withdraw at cap");

    assert_has_headroom("batch_withdraw at cap", cost, 2);
}

#[test]
fn a_full_ttl_sweep_keeps_headroom_on_every_limit() {
    let h = Harness::new();
    let ids: std::vec::Vec<u64> = (0..MAX_BATCH_SIZE)
        .map(|_| h.create_simple(100 * ONE, YEAR))
        .collect();

    h.client.batch_extend_ttl(&h.ids(&ids));
    let cost = report(&h, "batch_extend_ttl at cap");

    assert_has_headroom("batch_extend_ttl at cap", cost, 2);
}

/// Per-item cost must be flat. If it were not — if some hidden collection were
/// being scanned — the cap would be meaningless and the contract would fail
/// under load exactly as the existing MVPs do.
#[test]
fn batch_cost_grows_linearly_and_not_faster() {
    let h = Harness::new();
    let ids: std::vec::Vec<u64> = (0..MAX_BATCH_SIZE)
        .map(|_| h.create_simple(100 * ONE, 100 * DAY))
        .collect();

    let mut measurements = std::vec::Vec::new();
    for size in [1usize, 4, 8, MAX_BATCH_SIZE as usize] {
        h.advance(DAY);
        h.client.batch_withdraw(&h.recipient, &h.ids(&ids[..size]));
        let cost = report(&h, &std::format!("batch_withdraw({size})"));
        measurements.push((size as u32, cost));
    }

    let (_, base) = measurements[0];
    for &(size, cost) in &measurements[1..] {
        assert!(
            cost.footprint <= base.footprint * size,
            "footprint grew faster than linearly: {size} items took {} vs {} for one",
            cost.footprint,
            base.footprint,
        );
    }
}

/// The contract holds no per-user index, so a treasury's hundredth stream costs
/// exactly what its first did. This is the property the "no on-chain discovery"
/// decision buys, stated as a test — and it is precisely what the existing
/// implementations get wrong.
#[test]
fn cost_is_independent_of_how_many_streams_exist() {
    let h = Harness::new();

    // Skip the very first call: it also creates the instance entry and the
    // contract's own token balance entry, so it is one entry dearer than every
    // call after it. That is a fixed one-off, not growth.
    h.create_simple(100 * ONE, 100 * DAY);

    let early_id = h.create_simple(100 * ONE, 100 * DAY);
    let early_create = report(&h, "create #2");

    for _ in 0..150 {
        h.create_simple(10 * ONE, 100 * DAY);
    }
    let late_id = h.create_simple(100 * ONE, 100 * DAY);
    let late_create = report(&h, "create #153");

    assert_eq!(
        early_create.footprint, late_create.footprint,
        "creation footprint grew with the number of existing streams",
    );
    assert_eq!(early_create.writes, late_create.writes);
    assert_eq!(early_create.event_bytes, late_create.event_bytes);

    // Warm up: the first withdrawal to a given recipient also creates their
    // token balance entry, so it is one write dearer than every later one.
    // Burn that one-off on a throwaway stream to compare like with like.
    let warmup = h.create_simple(100 * ONE, 100 * DAY);
    h.advance(30 * DAY);
    h.client.withdraw(&warmup, &None);

    h.client.withdraw(&early_id, &None);
    let early = report(&h, "withdraw from #2");
    h.client.withdraw(&late_id, &None);
    let late = report(&h, "withdraw from #153");

    assert_eq!(
        early.footprint, late.footprint,
        "withdrawal footprint grew with the number of existing streams",
    );
    assert_eq!(early.writes, late.writes);
}

/// The event budget is the limit people forget: a batch emitting one event per
/// stream can exhaust event bytes before it exhausts entries. Measure the
/// per-event cost so the cap can be re-derived if the payload ever grows.
#[test]
fn the_event_budget_is_not_the_binding_constraint_at_the_cap() {
    let h = Harness::new();
    let ids: std::vec::Vec<u64> = (0..MAX_BATCH_SIZE)
        .map(|_| h.create_simple(100 * ONE, 100 * DAY))
        .collect();
    h.advance(30 * DAY);

    h.client.batch_withdraw(&h.recipient, &h.ids(&ids));
    let cost = report(&h, "batch events");

    let per_event = cost.event_bytes / MAX_BATCH_SIZE;
    let max_events_by_budget = EVENT_BYTES_LIMIT / per_event.max(1);
    std::println!(
        "per-event ~{per_event} bytes; event budget alone would allow ~{max_events_by_budget} \
         streams per batch (cap is {MAX_BATCH_SIZE})",
    );

    assert!(
        max_events_by_budget > MAX_BATCH_SIZE,
        "the event budget, not the entry count, is what bounds the batch",
    );
}

/// Enforce a hard ceiling on `batch_withdraw` budget usage. This is the reviewed
/// threshold: any optimization that breaks these numbers is a regression.
#[rustfmt::skip]
#[test]
fn batch_withdraw_budget_regression() {
    let h = Harness::new();

    let typical_size = 4usize;
    let max_size = MAX_BATCH_SIZE as usize;
    let total_streams = typical_size + max_size;

    let ids: Vec<u64> = (0..total_streams)
        .map(|_| h.create_simple(100 * ONE, 100 * DAY))
        .collect();

    h.advance(30 * DAY);

    // Empty batch
    assert_eq!(
        h.client.try_batch_withdraw(&h.recipient, &h.ids(&[])),
        Err(Ok(Error::EmptyBatch))
    );
    let empty = report(&h, "batch_withdraw(0)");

    // Typical batch
    h.client.batch_withdraw(&h.recipient, &h.ids(&ids[..typical_size]));
    let typical = report(&h, "batch_withdraw(4)");

    // Max batch
    h.client.batch_withdraw(&h.recipient, &h.ids(&ids[typical_size..]));
    let max = report(&h, "batch_withdraw(16)");

    // Thresholds established by measurement.
    // They must never increase significantly without a design review.
    assert!(
        empty.footprint <= 30,
        "empty batch footprint regression: {}",
        empty.footprint
    );
    assert!(
        empty.instructions <= 15_000_000,
        "empty batch CPU regression: {}",
        empty.instructions
    );

    assert!(
        typical.footprint <= 100,
        "typical batch footprint regression: {}",
        typical.footprint
    );
    assert!(
        typical.instructions <= 80_000_000,
        "typical batch CPU regression: {}",
        typical.instructions
    );

    assert!(
        max.footprint <= 350,
        "max batch footprint regression: {}",
        max.footprint
    );
    assert!(
        max.instructions <= 350_000_000,
        "max batch CPU regression: {}",
        max.instructions
    );
}
