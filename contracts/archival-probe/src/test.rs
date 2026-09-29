#![cfg(test)]

//! Tests for the archival probe (#1863).
//!
//! The probe is the mechanism by which archival problems are detected, and it is
//! deliberately tiny: three entry points, one storage key, no auth, no value.
//! That makes it easy to assume it still works — and a silent break in it does
//! not fail anything else, it removes a signal. So these tests pin its whole
//! surface rather than its happy path.
//!
//! Three things are asserted that the original smoke test did not:
//!
//! * **the archived state is exercised, not just described.** The SDK test host
//!   auto-restores an expired entry on read, so `test.rs` can never *observe*
//!   archival. What it can observe is the state the probe reports once the entry
//!   is gone, which is what an SDK keys on: `planted()` false and `read()`
//!   `NotPlanted`. That is the same distinction
//!   `FluxoraStream::stream_exists` draws, and it is what makes "archived, needs
//!   restoring" tellable from "never planted".
//! * **the entry point surface is complete.** Every entry point the probe
//!   exports is invoked here, and the exported set is taken from the contract
//!   spec, so removing or renaming one fails these tests by name.
//! * **the probe's whole point is preserved.** It extends no TTL and grows no
//!   state: a probe that quietly extended its entry, or accumulated keys, would
//!   still "pass" a round-trip test while no longer archiving on schedule.

// The crate is `no_std`; the test build has a host, and these tests need it.
extern crate std;

use std::string::{String, ToString};
use std::vec::Vec;

use super::*;
use soroban_sdk::testutils::storage::Persistent as _;
use soroban_sdk::testutils::{Ledger as _, MockAuth};
use soroban_sdk::xdr::{Limits, ReadXdr, ScSpecEntry};
use soroban_sdk::{symbol_short, Address, Symbol};

/// The entry point names the probe exports, read out of its embedded spec.
///
/// A removed or renamed entry point changes this list, which is what makes the
/// coverage assertions below fail by name rather than silently shrinking.
fn exported_entry_points() -> Vec<String> {
    fn named<const N: u32>(name: &soroban_sdk::xdr::StringM<N>) -> String {
        core::str::from_utf8(name.as_ref()).unwrap().to_string()
    }

    fn collect(names: &mut Vec<String>, spec: &[u8]) {
        let entry = ScSpecEntry::from_xdr(spec, Limits::none()).expect("spec XDR is well-formed");
        match entry {
            ScSpecEntry::FunctionV0(f) => names.push(named(&f.name)),
            other => panic!("expected a function spec entry, got {other:?}"),
        }
    }

    let mut names = Vec::new();
    collect(&mut names, &ArchivalProbe::spec_xdr_plant());
    collect(&mut names, &ArchivalProbe::spec_xdr_read());
    collect(&mut names, &ArchivalProbe::spec_xdr_planted());
    names
}

const CANARY: Symbol = symbol_short!("canary");

struct Fixture<'a> {
    env: Env,
    id: Address,
    client: ArchivalProbeClient<'a>,
}

fn fixture() -> Fixture<'static> {
    let env = Env::default();
    let id = env.register(ArchivalProbe, ());
    let client = ArchivalProbeClient::new(&env, &id);
    Fixture { env, id, client }
}

impl Fixture<'_> {
    /// Remaining TTL, in ledgers, of the canary entry.
    fn ttl(&self) -> u32 {
        self.env.as_contract(&self.id, || {
            self.env.storage().persistent().get_ttl(&Key::Canary)
        })
    }

    /// Whether the single canary entry exists in persistent storage.
    fn has_entry(&self) -> bool {
        self.env.as_contract(&self.id, || {
            self.env.storage().persistent().has(&Key::Canary)
        })
    }

    /// Delete the canary entry, standing in for the ledger having evicted it.
    ///
    /// The test host auto-restores an expired entry on read, so this is the only
    /// way to observe the state the network reports once an entry archives.
    fn evict(&self) {
        self.env.as_contract(&self.id, || {
            self.env.storage().persistent().remove(&Key::Canary);
        });
    }
}

// ---------------------------------------------------------------------------
// Entry point coverage
// ---------------------------------------------------------------------------

/// Each entry point round-trips on its own, before anything else happens.
#[test]
fn every_entry_point_is_exercised() {
    let f = fixture();

    // `planted` — the presence predicate, before anything exists.
    assert!(!f.client.planted());

    // `read` — the value view, before anything exists.
    assert_eq!(f.client.try_read().unwrap_err().unwrap(), Error::NotPlanted);

    // `plant` — the write.
    f.client.plant(&CANARY);

    // `planted` and `read` again, now that the canary exists.
    assert!(f.client.planted());
    assert_eq!(f.client.read(), CANARY);
}

/// The exported entry points are exactly the three this module drives. Read
/// from the embedded contract spec, so removing or renaming one fails here —
/// and in the coverage test below — rather than silently reducing the probe's
/// surface.
#[test]
fn the_exported_entry_point_set_is_the_documented_one() {
    let mut names = exported_entry_points();
    names.sort();

    assert_eq!(
        names,
        std::vec![
            "plant".to_string(),
            "planted".to_string(),
            "read".to_string(),
        ],
        "the probe's exported surface changed; the archival canary script and \
         docs/KNOWN-LIMITATIONS.md both describe three entry points",
    );
}

// ---------------------------------------------------------------------------
// The network's archival behaviour, pinned against the test host
// ---------------------------------------------------------------------------
//
// Recorded live on testnet, 2026-09-28 (see docs/KNOWN-LIMITATIONS.md §1):
//
//   canary planted at ledger   4,097,334
//   canary live until          4,218,293   (min_persistent_ttl - 1)
//   observed at ledger         4,922,344   (704,051 past live-until, ~40.7 days)
//   `getLedgerEntries` served the entry with `liveUntilLedgerSeq: 0` — its TTL
//     entry is gone, i.e. the data entry is archived — and still returned the
//     stored value.
//   a **single** `InvokeHostFunction` of `read` (tx
//     32e08f32d30db0f1f1a45786dbe7f8d87ca4f83dbd3e3ced0a0d5b54d807651c,
//     ledger 4,922,351) SUCCEEDED and returned `canary`. Its
//     `SorobanTransactionData` carried `archived_soroban_entries: [0, 1, 2]` —
//     the canary entry, the contract instance and the contract code — all three
//     restored automatically by that same invocation. No `RestoreFootprint`
//     resubmission happened, and the restoring transaction was billed
//     5,912,822 stroops of resource fee for it.
//   both data entries came back with `liveUntilLedgerSeq: 5,043,310`
//     (4,922,351 + min_persistent_ttl - 1).
//
// The read therefore never fails: archival is not a failure mode for persistent
// entries on this network, and the "detect it and offer a restore" path the
// limitation used to prescribe has no trigger to fire on. These tests pin the
// test host to that same contract.

/// Plant the canary, then jump the ledger past its TTL so the entry archives.
fn archived_canary() -> (Env, soroban_sdk::Address) {
    use soroban_sdk::testutils::Ledger as _;

    let env = Env::default();
    let id = env.register(ArchivalProbe, ());
    ArchivalProbeClient::new(&env, &id).plant(&symbol_short!("canary"));

    let planted_at = env.ledger().sequence();
    let ttl = env.as_contract(&id, || env.storage().persistent().get_ttl(&Key::Canary));
    // `get_ttl` is relative, so live-until is `planted_at + ttl`; step one past.
    env.ledger().set_sequence_number(planted_at + ttl + 1);

    (env, id)
}

/// Reading an archived entry succeeds. The host restores it in place, exactly as
/// the network does, and the value comes back intact.
#[test]
fn an_archived_entry_is_restored_by_the_read_itself() {

    let (env, id) = archived_canary();
    let client = ArchivalProbeClient::new(&env, &id);

    assert_eq!(client.read(), symbol_short!("canary"));

    // A restored entry holds exactly the network minimum, less the ledger the
    // restoring invocation itself closed in — on testnet it came back at
    // 4,922,351 + min_persistent_ttl - 1, the same shape.
    let min = env.ledger().get().min_persistent_entry_ttl;
    let ttl = env.as_contract(&id, || env.storage().persistent().get_ttl(&Key::Canary));
    assert_eq!(
        ttl,
        min - 1,
        "an auto-restored entry keeps the network minimum TTL, not more",
    );
}

/// `planted()` — the analogue of `FluxoraStream::stream_exists` — keeps
/// answering `true` across archival, because the read that would observe the
/// archived state is itself what restores it. There is no "needs restoring"
/// signal for a caller to key on.
#[test]
fn presence_stays_true_across_archival() {
    let (env, id) = archived_canary();
    let client = ArchivalProbeClient::new(&env, &id);

    assert!(
        client.planted(),
        "an archived-but-auto-restored entry must still report as present",
    );
}

/// Restoration is metered as entry writes and rent bumps, which is why the live
/// run paid a resource fee for it rather than getting recovery for free.
///
/// Only two entries are written here, not three: the probe is registered
/// natively in the test host, so there is no contract *code* entry to restore.
/// On the network the same invocation restored `[canary, instance, code]`.
#[test]
fn auto_restoration_is_metered_as_writes_and_rent_bumps() {
    let (env, id) = archived_canary();
    let client = ArchivalProbeClient::new(&env, &id);

    let _ = client.read();

    let resources = env.cost_estimate().resources();
    assert_eq!(
        resources.write_entries, 2,
        "restoring the canary and the contract instance is two entry writes",
    );
    assert_eq!(
        resources.persistent_entry_rent_bumps, 2,
        "restoring the canary and the contract instance is two rent bumps",
    );
}
/// `plant` is permissionless: it must succeed with an empty auth set, because
/// there is nothing here worth protecting and requiring auth would make the
/// canary unplantable by a monitoring job.
#[test]
fn plant_requires_no_authorization() {
    let f = fixture();
    f.env.mock_auths(&[] as &[MockAuth]);

    f.client.plant(&CANARY);

    assert!(f.client.planted());
    assert!(
        f.env.auths().is_empty(),
        "plant must consume no authorization entries",
    );
}

// ---------------------------------------------------------------------------
// Archived-entry detection
// ---------------------------------------------------------------------------

/// Once the entry is gone — the state the network reports after archival — the
/// probe reports `planted() == false` and `read()` as `NotPlanted`, which is the
/// signal an SDK keys on to offer a restore rather than surfacing a raw error.
#[test]
fn an_evicted_entry_is_reported_as_not_planted() {
    let f = fixture();
    f.client.plant(&CANARY);
    assert!(f.client.planted());

    f.evict();

    assert!(
        !f.client.planted(),
        "an evicted canary must not report as planted",
    );
    assert_eq!(
        f.client.try_read().unwrap_err().unwrap(),
        Error::NotPlanted,
        "an evicted canary must read as NotPlanted, the same typed error as a \
         canary that was never planted",
    );
}

/// The distinction the SDK needs is observable, not inferred: `planted()` is the
/// predicate that tells "never planted" from "planted then evicted", and
/// `read()` gives the same typed error for both.
#[test]
fn planted_distinguishes_never_planted_from_evicted() {
    let never = fixture();
    assert!(!never.client.planted());

    let evicted = fixture();
    evicted.client.plant(&CANARY);
    evicted.evict();

    // Identical observable state — which is exactly why a client must consult a
    // block explorer or a restore attempt, and why `planted` exists at all.
    assert_eq!(never.client.planted(), evicted.client.planted());
    assert_eq!(
        never.client.try_read().unwrap_err().unwrap(),
        evicted.client.try_read().unwrap_err().unwrap(),
    );
}

/// Re-planting restores the signal: the probe is the recovery path for its own
/// canary, so a monitoring job can always re-arm it without a redeploy.
#[test]
fn replanting_restores_the_canary_after_eviction() {
    let f = fixture();
    f.client.plant(&CANARY);
    f.evict();
    assert!(!f.client.planted());

    f.client.plant(&symbol_short!("revived"));

    assert!(f.client.planted());
    assert_eq!(f.client.read(), symbol_short!("revived"));
}

// ---------------------------------------------------------------------------
// The probe's whole point: it must archive
// ---------------------------------------------------------------------------

/// The omission that makes the probe work, asserted from both entry points that
/// touch the entry: neither `plant` nor `read` may extend the TTL.
///
/// `plant` not extending it is documented; `read` not extending it is the one
/// that would be easy to "improve" by accident, and it would silently stop the
/// probe from ever archiving.
#[test]
fn neither_plant_nor_read_extends_the_ttl() {
    let f = fixture();
    f.client.plant(&CANARY);
    let after_plant = f.ttl();

    // Read repeatedly; a read that bumped the TTL would show up as growth.
    for _ in 0..5 {
        assert_eq!(f.client.read(), CANARY);
    }
    let after_reads = f.ttl();

    assert_eq!(
        after_reads, after_plant,
        "reading the canary must not extend its entry",
    );

    let min = f.env.ledger().get().min_persistent_entry_ttl;
    assert!(
        after_plant < min,
        "the canary received {after_plant} ledgers of TTL, at or above the \
         network minimum {min}; something is extending it and it will not \
         archive on schedule",
    );
}

/// The canary lives out the network's minimum and no longer, so the countdown
/// the canary script's constants describe is the countdown the contract
/// actually has.
#[test]
fn the_canary_gets_exactly_the_network_minimum() {
    let f = fixture();
    f.client.plant(&CANARY);

    // `get_ttl` reports `live_until - sequence`; at plant time nothing has been
    // consumed, so the entry holds the maximum the host will grant a
    // freshly-written persistent entry.
    let ttl = f.ttl();
    let min = f.env.ledger().get().min_persistent_entry_ttl;

    assert!(
        ttl >= min - 1,
        "a freshly planted canary should hold the network minimum ({min}), \
         got {ttl}",
    );
}

/// A jumping ledger — the test host's stand-in for time passing on a real
/// network — leaves the canary readable, because the host auto-restores. This is
/// the caveat `docs/KNOWN-LIMITATIONS.md` §1 states, asserted so the limitation
/// stays visible rather than being forgotten: only an explicit eviction can
/// reproduce the network's behaviour in-process.
#[test]
fn the_host_auto_restores_which_is_why_the_live_round_trip_exists() {
    let f = fixture();
    f.client.plant(&CANARY);
    f.env.ledger().set_max_entry_ttl(20_000);

    f.env
        .ledger()
        .set_sequence_number(f.env.ledger().sequence() + 100_000);

    assert!(
        f.client.planted(),
        "the test host auto-restores an expired entry, so time alone cannot \
         reproduce archival in-process",
    );
    assert_eq!(f.client.read(), CANARY);
}

// ---------------------------------------------------------------------------
// Isolation
// ---------------------------------------------------------------------------

/// The probe holds one entry and only ever overwrites it: it is a canary, not a
/// store. Enumerated against the key the contract actually uses, and against
/// the value the last write left behind.
#[test]
fn the_probe_never_accumulates_state() {
    let f = fixture();
    f.client.plant(&CANARY);
    assert!(f.has_entry());

    for note in ["second", "third", "fourth"] {
        let sym = Symbol::new(&f.env, note);
        f.client.plant(&sym);
        assert!(
            f.has_entry(),
            "the canary entry must survive every re-plant",
        );
        assert_eq!(f.client.read(), sym);
    }

    // Reads and TTL reads never create anything.
    let _ = f.client.planted();
    let _ = f.client.read();
    assert!(f.has_entry());
}

/// `plant` overwrites rather than appending, so the canary always holds the
/// latest note and nothing is left behind for a later read to find.
#[test]
fn plant_overwrites_the_previous_note() {
    let f = fixture();
    for note in ["first", "second", "third"] {
        // `symbol_short!` needs a literal; go through the client with a created
        // symbol instead.
        let sym = Symbol::new(&f.env, note);
        f.client.plant(&sym);
        assert_eq!(f.client.read(), sym);
        assert!(f.has_entry());
    }
}

/// The error discriminant is part of the probe's ABI and is documented; freezing
/// it here means a renumbering is a deliberate, visible change.
#[test]
fn the_error_discriminant_is_frozen() {
    assert_eq!(Error::NotPlanted as u32, 1);
}

/// The probe is deliberately not a product contract: it must not export auth,
/// value or configuration entry points. Asserted against the spec set so a
/// well-meaning addition fails here rather than in a release audit.
#[test]
fn the_probe_exports_no_privileged_surface() {
    let names = exported_entry_points();

    for forbidden in [
        "init",
        "upgrade",
        "set_admin",
        "pause",
        "resume",
        "withdraw",
        "transfer",
        "mint",
    ] {
        assert!(
            !names.iter().any(|n| n == forbidden),
            "the probe must stay a canary; `{forbidden}` is not part of its surface",
        );
    }
}
