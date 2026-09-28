#![cfg(test)]

use super::*;
use soroban_sdk::symbol_short;

#[test]
fn plant_and_read_round_trip() {
    let env = Env::default();
    let id = env.register(ArchivalProbe, ());
    let client = ArchivalProbeClient::new(&env, &id);

    assert!(!client.planted());
    assert_eq!(client.try_read().unwrap_err().unwrap(), Error::NotPlanted);

    client.plant(&symbol_short!("canary"));
    assert!(client.planted());
    assert_eq!(client.read(), symbol_short!("canary"));
}

/// The probe must never extend its own TTL — that omission is what makes it
/// archive on the network's schedule instead of the contract's.
#[test]
fn the_probe_does_not_extend_its_own_ttl() {
    use soroban_sdk::testutils::storage::Persistent as _;
    use soroban_sdk::testutils::Ledger as _;

    let env = Env::default();
    let id = env.register(ArchivalProbe, ());
    ArchivalProbeClient::new(&env, &id).plant(&symbol_short!("canary"));

    let ttl = env.as_contract(&id, || env.storage().persistent().get_ttl(&Key::Canary));
    let min = env.ledger().get().min_persistent_entry_ttl;

    assert!(
        ttl < min,
        "probe TTL {ttl} exceeds the network minimum {min}; something is \
         extending it and the probe will not archive on schedule",
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
    use soroban_sdk::testutils::storage::Persistent as _;
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
    use soroban_sdk::testutils::storage::Persistent as _;
    use soroban_sdk::testutils::Ledger as _;

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
