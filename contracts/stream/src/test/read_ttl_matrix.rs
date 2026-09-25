//! Issue #1686 — which read entry points write, across the whole ABI.
//!
//! `read_methods_no_side_effects.rs` checks the TTL of one stream entry for
//! four of the views. This file closes the gaps the issue lists:
//!
//! * **Every** read entry point is classified, including `stream_exists` and
//!   `stream_count`, and the classification is pinned to `docs/ABI.md`: a view
//!   added to the ABI tables without a classification here fails
//!   [`abi_doc_classifies_every_read_entry_point`].
//! * The check is a snapshot of **all** ledger entries (data and `live_until`)
//!   before and after the call — stream entries, the instance entry, the
//!   contract code, and the token's entries — plus the host's count of written
//!   entries. A pure read must leave all of it bit-for-bit identical.
//! * The two maintenance entry points that are *supposed* to extend TTL are
//!   asserted to do exactly that: `live_until` moves forward, and nothing
//!   else — no entry's data — changes.
//! * [`detector_catches_a_read_that_bumps_ttl`] proves the snapshot check fires
//!   when a read path goes through `storage::load_stream` (which extends TTL)
//!   instead of `storage::peek_stream`, so converting a view from pure to
//!   writing fails this suite.
//!
//! # Why the fixture uses multi-year streams and ages the ledger
//!
//! A TTL bump on an entry whose TTL is already at its target is a no-op and
//! would be invisible to a snapshot. For an ordinary stream the target is
//! "remaining lifetime + buffer" (`storage::ttl_target_ledgers`), which shrinks
//! in lockstep with the entry's own TTL as time passes — so aging alone never
//! opens a gap. The fixture therefore creates streams long enough that their
//! target is clamped to the network maximum, then advances one hour (720
//! ledgers): the entries (and the instance) now sit below their target, so any
//! extension moves `live_until` and shows up. [`fixture`] asserts that gap
//! exists, so the pure-read tests can never silently become blind.
//!
//! One hour is far below every TTL the contract sets, so no entry comes near
//! expiry — reading an *expired* entry makes the test host auto-restore it,
//! which is a write of its own (see `ttl.rs`).

use soroban_sdk::xdr::{LedgerEntry, LedgerKey};

use super::common::*;
use crate::storage;

/// Pure views: must write nothing, TTL included. Keep in sync with the
/// "Views" table in `docs/ABI.md`.
const PURE_READS: &[&str] = &[
    "get_stream",
    "withdrawable_of",
    "vested_of",
    "refundable_of",
    "stream_count",
    "stream_exists",
];

/// Read-like maintenance calls whose whole job is to extend TTL. They must
/// never change entry data. Keep in sync with the "Maintenance" table.
const TTL_EXTENDING_READS: &[&str] = &["extend_stream_ttl", "batch_extend_ttl"];

const HOUR: u64 = 3_600;
const MISSING_ID: u64 = 999;

type Entries = std::vec::Vec<(
    std::boxed::Box<LedgerKey>,
    (std::boxed::Box<LedgerEntry>, Option<u32>),
)>;

/// Every ledger entry the test host holds, with its `live_until` ledger.
fn entries(h: &Harness) -> Entries {
    h.env.to_ledger_snapshot().ledger_entries
}

/// Two live multi-year streams, then one hour of ledger time, so that any TTL
/// bump on them is visible (see module docs).
fn fixture() -> (Harness<'static>, u64, u64) {
    let h = Harness::new();
    let a = h.create_simple(1_000 * ONE, 10 * YEAR);
    let b = h.create_simple(500 * ONE, 5 * YEAR);
    h.advance(HOUR);
    for id in [a, b] {
        assert!(
            h.ttl_of(id) < h.max_achievable_ttl(),
            "fixture precondition: stream {id} must sit below its TTL target, \
             or a TTL bump would be invisible to the snapshot",
        );
    }
    (h, a, b)
}

/// Human-readable list of the entries that differ between two snapshots.
fn describe_diff(before: &Entries, after: &Entries) -> std::string::String {
    let mut out = std::string::String::new();
    if before.len() != after.len() {
        out.push_str(&std::format!(
            "entry count {} -> {}; ",
            before.len(),
            after.len()
        ));
    }
    for ((kb, (eb, lb)), (ka, (ea, la))) in before.iter().zip(after.iter()) {
        if kb != ka || eb != ea || lb != la {
            out.push_str(&std::format!(
                "[key {:?}: data_changed={} live_until {:?} -> {:?}] ",
                kb,
                eb != ea,
                lb,
                la
            ));
        }
    }
    out
}

/// Run `call` and assert it wrote nothing: no written entries reported by the
/// host, and every ledger entry (data and TTL) identical before and after.
fn assert_pure(h: &Harness, label: &str, call: impl FnOnce()) {
    let before = entries(h);
    call();
    let writes = h.env.cost_estimate().resources().write_entries;
    let after = entries(h);
    assert_eq!(
        writes, 0,
        "{label}: expected no storage writes, host reported {writes}"
    );
    assert!(
        before == after,
        "{label}: documented as pure but changed storage: {}",
        describe_diff(&before, &after)
    );
}

/// Assert `before -> after` changed nothing but TTLs, and only forwards.
/// Returns how many entries had their `live_until` extended.
fn assert_only_ttl_extended(label: &str, before: &Entries, after: &Entries) -> usize {
    assert_eq!(
        before.len(),
        after.len(),
        "{label}: entries were added or removed"
    );
    let mut extended = 0;
    for ((kb, (eb, lb)), (ka, (ea, la))) in before.iter().zip(after.iter()) {
        assert_eq!(kb, ka, "{label}: entry set changed");
        assert_eq!(eb, ea, "{label}: entry data changed for {kb:?}");
        if lb != la {
            assert!(la > lb, "{label}: live_until moved backwards for {kb:?}");
            extended += 1;
        }
    }
    extended
}

// --- Pure reads: no write, no TTL bump -------------------------------------

#[test]
fn get_stream_writes_nothing() {
    let (h, a, b) = fixture();
    for id in [a, b] {
        assert_pure(&h, "get_stream(existing)", || {
            h.client.get_stream(&id);
        });
    }
    assert_pure(&h, "get_stream(missing)", || {
        assert!(h.client.try_get_stream(&MISSING_ID).is_err());
    });
}

#[test]
fn withdrawable_of_writes_nothing() {
    let (h, a, b) = fixture();
    for id in [a, b] {
        assert_pure(&h, "withdrawable_of(existing)", || {
            h.client.withdrawable_of(&id);
        });
    }
    assert_pure(&h, "withdrawable_of(missing)", || {
        assert!(h.client.try_withdrawable_of(&MISSING_ID).is_err());
    });
}

#[test]
fn vested_of_writes_nothing() {
    let (h, a, b) = fixture();
    for id in [a, b] {
        assert_pure(&h, "vested_of(existing)", || {
            h.client.vested_of(&id);
        });
    }
    assert_pure(&h, "vested_of(missing)", || {
        assert!(h.client.try_vested_of(&MISSING_ID).is_err());
    });
}

#[test]
fn refundable_of_writes_nothing() {
    let (h, a, b) = fixture();
    for id in [a, b] {
        assert_pure(&h, "refundable_of(existing)", || {
            h.client.refundable_of(&id);
        });
    }
    assert_pure(&h, "refundable_of(missing)", || {
        assert!(h.client.try_refundable_of(&MISSING_ID).is_err());
    });
}

#[test]
fn stream_count_writes_nothing() {
    let (h, _, _) = fixture();
    assert_pure(&h, "stream_count", || {
        assert_eq!(h.client.stream_count(), 2);
    });
}

#[test]
fn stream_exists_writes_nothing() {
    let (h, a, _) = fixture();
    assert_pure(&h, "stream_exists(existing)", || {
        assert!(h.client.stream_exists(&a));
    });
    assert_pure(&h, "stream_exists(missing)", || {
        assert!(!h.client.stream_exists(&MISSING_ID));
    });
}

/// Every pure read, called back to back, still leaves storage untouched.
#[test]
fn all_pure_reads_together_write_nothing() {
    let (h, a, _) = fixture();
    assert_pure(&h, "all pure reads", || {
        h.client.get_stream(&a);
        h.client.withdrawable_of(&a);
        h.client.vested_of(&a);
        h.client.refundable_of(&a);
        h.client.stream_count();
        h.client.stream_exists(&a);
    });
}

// --- TTL-extending reads: bump TTL, never data ----------------------------

#[test]
fn extend_stream_ttl_extends_ttl_and_changes_no_data() {
    let (h, a, b) = fixture();
    let ttl_a_before = h.ttl_of(a);
    let ttl_b_before = h.ttl_of(b);

    let before = entries(&h);
    let funded = h.client.extend_stream_ttl(&a);
    let after = entries(&h);

    let extended = assert_only_ttl_extended("extend_stream_ttl", &before, &after);
    assert!(extended >= 1, "extend_stream_ttl extended nothing");
    assert!(
        h.ttl_of(a) > ttl_a_before,
        "target stream TTL was not extended"
    );
    assert_eq!(
        h.ttl_of(a),
        funded,
        "returned ledgers must match the new TTL"
    );
    assert_eq!(
        h.ttl_of(b),
        ttl_b_before,
        "an untouched stream's TTL changed"
    );
}

#[test]
fn batch_extend_ttl_extends_ttl_and_changes_no_data() {
    let (h, a, b) = fixture();
    let ttl_a_before = h.ttl_of(a);
    let ttl_b_before = h.ttl_of(b);

    let before = entries(&h);
    // An unknown id is skipped, not written.
    let count = h.client.batch_extend_ttl(&h.ids(&[a, b, MISSING_ID]));
    let after = entries(&h);

    assert_eq!(count, 2);
    let extended = assert_only_ttl_extended("batch_extend_ttl", &before, &after);
    assert!(
        extended >= 2,
        "batch_extend_ttl extended {extended} entries, expected both streams"
    );
    assert!(h.ttl_of(a) > ttl_a_before);
    assert!(h.ttl_of(b) > ttl_b_before);
}

// --- The detector itself ----------------------------------------------------

/// If a view were switched from `peek_stream` to `load_stream`, it would bump
/// TTL. Doing exactly that by hand must trip the snapshot comparison
/// `assert_pure` relies on — otherwise the pure-read tests prove nothing.
#[test]
fn detector_catches_a_read_that_bumps_ttl() {
    let (h, a, _) = fixture();
    let before = entries(&h);
    h.env.as_contract(&h.contract_id, || {
        storage::load_stream(&h.env, a).unwrap();
    });
    let after = entries(&h);
    assert!(
        before != after,
        "load_stream bumps TTL, so the snapshot must differ; the detector is blind"
    );
    assert!(assert_only_ttl_extended("load_stream", &before, &after) >= 1);
}

/// `peek_stream` — the helper every view uses — is the pure counterpart.
#[test]
fn peek_stream_writes_nothing() {
    let (h, a, _) = fixture();
    let before = entries(&h);
    h.env.as_contract(&h.contract_id, || {
        storage::peek_stream(&h.env, a).unwrap();
    });
    assert!(before == entries(&h), "peek_stream must not touch storage");
}

// --- Documentation is the source of truth ----------------------------------

fn abi_doc() -> std::string::String {
    let path = std::format!("{}/../../docs/ABI.md", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"))
}

/// The ABI table row for `name`, e.g. "| `get_stream(stream_id)` | ... |".
fn abi_row<'d>(doc: &'d str, name: &str) -> &'d str {
    let needle = std::format!("| `{name}(");
    doc.lines()
        .find(|l| l.starts_with(&needle))
        .unwrap_or_else(|| panic!("docs/ABI.md has no table row for `{name}`"))
}

/// Function names listed in the ABI table under `start`.
fn abi_section_functions(doc: &str, start: &str) -> std::vec::Vec<std::string::String> {
    let from = doc
        .find(start)
        .unwrap_or_else(|| panic!("docs/ABI.md: no section {start}"));
    let section = &doc[from..];
    let section = &section[..section.find("\n### ").unwrap_or(section.len())];
    section
        .lines()
        .filter_map(|l| l.strip_prefix("| `"))
        .filter_map(|l| l.split('`').next())
        .filter(|name| name.contains('('))
        .filter_map(|name| name.split('(').next())
        .map(std::string::String::from)
        .collect()
}

#[test]
fn abi_doc_states_ttl_behaviour_per_read_method() {
    let doc = abi_doc();
    for name in PURE_READS {
        let row = abi_row(&doc, name);
        assert!(
            row.ends_with("| no | no |"),
            "docs/ABI.md must mark `{name}` as not extending TTL and not writing: {row}"
        );
    }
    for name in TTL_EXTENDING_READS {
        let row = abi_row(&doc, name);
        assert!(
            row.contains("| **yes**"),
            "docs/ABI.md must mark `{name}` as extending TTL: {row}"
        );
    }
}

#[test]
fn abi_doc_classifies_every_read_entry_point() {
    let doc = abi_doc();
    let mut documented = abi_section_functions(&doc, "### Views");
    documented.extend(abi_section_functions(&doc, "### Maintenance"));
    documented.sort();
    let mut classified: std::vec::Vec<std::string::String> = PURE_READS
        .iter()
        .chain(TTL_EXTENDING_READS.iter())
        .map(|s| std::string::String::from(*s))
        .collect();
    classified.sort();
    assert_eq!(
        documented, classified,
        "every view / maintenance entry point in docs/ABI.md must be classified in this file"
    );
}
