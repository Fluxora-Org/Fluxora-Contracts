//! Issue #1860 — a stream's id is never reused after its stream is removed.
//!
//! The id allocator is an instance-level counter (`storage::next_stream_id`),
//! and `stream_ids.rs` proves it is monotonic across *failed* creates and
//! across terminal states. Neither of those removes a record: a terminal
//! operation rewrites the record in place, so the population only grows and the
//! allocator is never actually asked to skip a hole.
//!
//! A hole is exactly where a naive allocator would break. If ids were derived
//! from the population — or from "the highest record that still exists" — then
//! a stream whose entry was removed would hand its id to the next create, and a
//! client holding `stream_id = 3` from an event, a UI or a delegate grant would
//! silently find itself pointing at a different stream. That is a
//! fund-safety-grade ambiguity, not a cosmetic one.
//!
//! # Why the records are removed by hand
//!
//! On a real network a record disappears when its TTL runs out and the entry
//! archives: `stream_exists(id)` reports `false` while the id stays below the
//! counter (`docs/KNOWN-LIMITATIONS.md`). The SDK test host auto-restores an
//! archived entry on the next access, so *time alone cannot produce a hole in
//! tests* — `common.rs` says so explicitly, and
//! `test::stream_count_consistency` uses an explicit `persistent().remove(...)`
//! as the sanctioned stand-in for the same condition. This module does the
//! same, and states the invariant that has to survive it:
//!
//! **the id handed to the next create is a function of the counter alone, never
//! of which records happen to be present.**
//!
//! # Note on the pool invariant
//!
//! Removing a record deliberately strands the tokens it accounted for, so
//! `Harness::assert_pool_exact` is *expected* to fail once a record is gone and
//! is therefore only called before the first removal. Nothing here is a claim
//! about the pool.

use super::common::*;
use crate::{DataKey, Error, StreamStatus};

/// Delete a stream record outright, the way an archived entry looks to any
/// caller: absent from persistent storage, counter unchanged.
fn remove_record(h: &Harness, id: u64) {
    h.env.as_contract(&h.contract_id, || {
        h.env.storage().persistent().remove(&DataKey::Stream(id));
    });
    assert!(
        !h.client.stream_exists(&id),
        "id {id} should be gone after removal",
    );
    assert_eq!(
        h.client.try_get_stream(&id).unwrap_err().unwrap(),
        Error::StreamNotFound,
        "a removed record must be unreachable",
    );
}

// ---------------------------------------------------------------------------
// The invariant, stated directly
// ---------------------------------------------------------------------------

/// The narrowest statement of the property: remove a record, then create. The
/// new stream must not land on the removed id.
#[test]
fn removing_a_stream_record_does_not_make_its_id_available_again() {
    let h = Harness::new();
    let first = h.create_simple(10 * ONE, DAY);
    assert_eq!(first, 0);

    remove_record(&h, first);

    let second = h.create_simple(10 * ONE, DAY);
    assert_eq!(second, 1, "the allocator must skip the hole, not fill it",);
    assert_ne!(second, first, "an id must never be reissued");
    assert!(h.client.stream_exists(&second));
}

/// Removing the *newest* record is the tempting optimisation: a
/// "highest existing id + 1" allocator would hand out the same id again. It must
/// not — the counter is a one-way ratchet.
#[test]
fn removing_the_newest_record_does_not_rewind_the_counter() {
    let h = Harness::new();
    for _ in 0..3 {
        h.create_simple(10 * ONE, DAY);
    }
    assert_eq!(h.client.stream_count(), 3);

    remove_record(&h, 2);

    let next = h.create_simple(10 * ONE, DAY);
    assert_eq!(next, 3, "the counter must not rewind to the population");
    assert_ne!(next, 2);
    assert_eq!(h.client.stream_count(), 4);
}

/// Removing *every* record must not reset the counter to zero. This is the
/// strongest form of the property: the population is empty and the next id is
/// still the high-water mark.
#[test]
fn removing_every_record_does_not_reset_the_counter() {
    let h = Harness::new();
    const CREATED: u64 = 5;
    for _ in 0..CREATED {
        h.create_simple(10 * ONE, DAY);
    }
    h.assert_pool_exact();

    for id in 0..CREATED {
        remove_record(&h, id);
    }
    assert_eq!(
        h.client.stream_count(),
        CREATED,
        "the counter is the high-water mark, not the population",
    );

    for expected in CREATED..CREATED + 3 {
        let id = h.create_simple(10 * ONE, DAY);
        assert_eq!(
            id, expected,
            "ids must resume from the high-water mark after a full wipe",
        );
    }
    assert_eq!(h.client.stream_count(), CREATED + 3);
}

/// Holes in the middle are never filled: the ids either side must stay
/// unreachable forever, however many creates follow.
#[test]
fn ids_removed_from_the_middle_are_never_reissued() {
    let h = Harness::new();
    const CREATED: u64 = 6;
    for _ in 0..CREATED {
        h.create_simple(10 * ONE, DAY);
    }

    let holes = [1u64, 3, 4];
    for &hole in holes.iter() {
        remove_record(&h, hole);
    }

    let mut issued = std::vec::Vec::new();
    for _ in 0..10 {
        let id = h.create_simple(10 * ONE, DAY);
        assert!(
            !holes.contains(&id),
            "id {id} was removed and must never be handed out again",
        );
        assert!(
            id >= CREATED,
            "id {id} must come from above the high-water mark",
        );
        if let Some(&last) = issued.last() {
            assert!(id > last, "ids must stay strictly increasing");
        }
        issued.push(id);
    }

    assert_eq!(h.client.stream_count(), CREATED + issued.len() as u64);
}

/// The counter is what the allocator reads — not the population. Removing
/// records changes the population and must leave `stream_count()` untouched.
#[test]
fn the_counter_reports_the_high_water_mark_not_the_population() {
    let h = Harness::new();
    for _ in 0..4 {
        h.create_simple(10 * ONE, DAY);
    }

    let counter_before = h.client.stream_count();
    let population_before = (0..counter_before)
        .filter(|&id| h.client.stream_exists(&id))
        .count() as u64;
    assert_eq!(counter_before, population_before);

    remove_record(&h, 2);

    assert_eq!(
        h.client.stream_count(),
        counter_before,
        "removal must not shrink the counter",
    );
    let population_after = (0..counter_before)
        .filter(|&id| h.client.stream_exists(&id))
        .count() as u64;
    assert_eq!(population_after, population_before - 1);
    assert_ne!(
        h.client.stream_count(),
        population_after,
        "the two representations must be allowed to disagree — that is the hole",
    );
}

// ---------------------------------------------------------------------------
// Terminal states are not holes, and must not become any
// ---------------------------------------------------------------------------

/// A cancelled or depleted stream keeps its id permanently: terminal states
/// rewrite the record rather than removing it, so the id stays occupied by a
/// record that explains what happened to it.
#[test]
fn a_terminal_stream_keeps_its_id_and_its_record() {
    let h = Harness::new();

    let cancelled = h.create_simple(100 * ONE, 100 * DAY);
    let depleted = h.create_simple(10 * ONE, DAY);

    h.advance(10 * DAY);
    h.client.cancel(&cancelled);
    h.advance(DAY);
    h.client.withdraw(&depleted, &None);

    assert_eq!(h.get(cancelled).status, StreamStatus::Cancelled);
    assert_eq!(h.get(depleted).status, StreamStatus::Depleted);
    assert!(h.client.stream_exists(&cancelled));
    assert!(h.client.stream_exists(&depleted));

    // Neither terminal id is free.
    for expected in [2u64, 3] {
        let id = h.create_simple(10 * ONE, DAY);
        assert_eq!(id, expected);
        assert_ne!(id, cancelled);
        assert_ne!(id, depleted);
    }
}

/// Even after a terminal stream's record is *also* removed, its id stays spent
/// — removing a settled record is not a way to reclaim an id.
#[test]
fn removing_a_settled_record_does_not_reclaim_its_id() {
    let h = Harness::new();
    let id = h.create_simple(100 * ONE, 100 * DAY);

    h.advance(30 * DAY);
    h.client.cancel(&id);
    h.assert_pool_exact();

    remove_record(&h, id);

    let next = h.create_simple(10 * ONE, DAY);
    assert_eq!(next, 1, "a settled-then-removed id must stay spent");
}

// ---------------------------------------------------------------------------
// Property test
// ---------------------------------------------------------------------------

/// xorshift64*. Deterministic and seedable, so a failure replays from its seed
/// alone — the convention set by `test::invariants`.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

/// Generate interleaved creates and removals, and assert after every create
/// that the id just issued had never been issued before and does not sit in a
/// hole.
///
/// The bookkeeping is independent of the contract: this module remembers every
/// id it has ever seen and every id it has ever removed, so the assertion is
/// against the test's own ledger rather than against the contract's counter.
fn run_create_and_remove_sequence(seed: u64, steps: u32) {
    let h = Harness::new();
    let mut rng = Rng(seed);

    let mut issued: std::vec::Vec<u64> = std::vec::Vec::new();
    let mut removed: std::vec::Vec<u64> = std::vec::Vec::new();
    let mut live: std::vec::Vec<u64> = std::vec::Vec::new();

    for step in 0..steps {
        let action = rng.below(10);

        if action < 6 || live.is_empty() {
            // Create.
            let id = h.create_simple(10 * ONE, DAY);
            assert!(
                !issued.contains(&id),
                "seed {seed}, step {step}: id {id} was issued twice",
            );
            assert!(
                !removed.contains(&id),
                "seed {seed}, step {step}: id {id} was reissued after removal",
            );
            if let Some(&last) = issued.last() {
                assert!(
                    id > last,
                    "seed {seed}, step {step}: id {id} is not above the high-water mark {last}",
                );
            }
            issued.push(id);
            live.push(id);
        } else {
            // Remove one of the live records.
            let victim = live.remove(rng.below(live.len() as u64) as usize);
            remove_record(&h, victim);
            removed.push(victim);
        }

        assert_eq!(
            h.client.stream_count(),
            issued.len() as u64,
            "seed {seed}, step {step}: the counter must count every create ever, not the population",
        );
    }

    // Every hole stays a hole, and the counter never rewinds.
    for &hole in removed.iter() {
        assert!(
            !h.client.stream_exists(&hole),
            "seed {seed}: removed id {hole} must still be absent",
        );
        assert!(
            h.client.try_get_stream(&hole).is_err(),
            "seed {seed}: removed id {hole} must still be unreachable",
        );
    }
    assert_eq!(h.client.stream_count(), issued.len() as u64);
}

#[test]
fn randomized_creates_and_removals_never_reissue_an_id() {
    for seed in 0..64u64 {
        run_create_and_remove_sequence(
            0x5DEE_CE66_D1CE_4E5Bu64 ^ seed.wrapping_mul(0x9E37_79B9_7F4A_7C15),
            40,
        );
    }
}

#[test]
fn long_randomized_creates_and_removals_never_reissue_an_id() {
    for seed in 0..6u64 {
        run_create_and_remove_sequence(0xC0FF_EE00_1234_5678u64.wrapping_add(seed), 200);
    }
}

/// A record's id must stay spent even when the removal happens *between* two
/// creates that both succeed — the pattern a real archive-then-create looks
/// like when a keeper restores one stream and a sender opens another.
#[test]
fn an_id_removed_between_two_successful_creates_stays_spent() {
    let h = Harness::new();
    let a = h.create_simple(10 * ONE, DAY);
    let b = h.create_simple(10 * ONE, DAY);
    assert_eq!((a, b), (0, 1));

    remove_record(&h, a);
    remove_record(&h, b);

    // Two creates, both successful, over a population that is now empty.
    let c = h.create_simple(10 * ONE, DAY);
    let d = h.create_simple(10 * ONE, DAY);
    assert_eq!((c, d), (2, 3));
    assert!(!h.client.stream_exists(&a));
    assert!(!h.client.stream_exists(&b));
    assert!(h.client.stream_exists(&c));
    assert!(h.client.stream_exists(&d));

    // And the removed ids are still spent after the new creates.
    let e = h.create_simple(10 * ONE, DAY);
    assert_eq!(e, 4);
    assert_eq!(h.get(e).deposited, 10 * ONE);
    assert!(!h.client.stream_exists(&a) && !h.client.stream_exists(&b));
}

/// Sanity check that the removal helper is doing what it claims: the entry is
/// gone from persistent storage while the counter still accounts for it. This
/// guards the rest of the module against a helper that silently no-ops.
#[test]
fn the_removal_helper_actually_removes_the_entry() {
    let h = Harness::new();
    let id = h.create_simple(10 * ONE, DAY);

    let in_storage = h.env.as_contract(&h.contract_id, || {
        h.env.storage().persistent().has(&DataKey::Stream(id))
    });
    assert!(in_storage, "the record must exist before removal");

    remove_record(&h, id);

    let in_storage = h.env.as_contract(&h.contract_id, || {
        h.env.storage().persistent().has(&DataKey::Stream(id))
    });
    assert!(
        !in_storage,
        "the removal helper must actually delete the entry"
    );
    assert_eq!(h.client.stream_count(), 1, "the counter is unchanged");
}
