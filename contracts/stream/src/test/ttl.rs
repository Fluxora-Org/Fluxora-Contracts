//! Stage 3 — TTL, rent, and archival.
//!
//! This is the file that separates a production streaming primitive from a
//! hackathon one. A stream running twelve months outlives its initial TTL, and
//! if the entry archives, the recipient's claim becomes unreadable until
//! somebody pays to restore it.
//!
//! # What the test host can and cannot prove
//!
//! The SDK's test host runs storage in *recording* mode, where reading an
//! expired persistent entry triggers `handle_maybe_expired_entry`: the entry is
//! restored in place with its data intact and its TTL reset to
//! `min_persistent_entry_ttl`. That mirrors the on-network outcome of a
//! `RestoreFootprint` operation, so these tests genuinely prove **data survives
//! the archive/restore boundary with balances intact**.
//!
//! What they cannot reproduce is the client-side dance on a real network, where
//! the transaction *fails first* and the caller must resubmit with a restore
//! footprint. That step has no unit-test surface and belongs in the testnet
//! exercise in stage 4.
//!
//! One useful consequence of the host's behaviour: an entry that has been
//! through an auto-restore has a TTL of exactly `min_persistent_entry_ttl - 1`,
//! which is far below anything this contract ever sets. [`was_restored`] uses
//! that as a detector for "this entry archived".

#[test]
fn seconds_to_ledgers_edge_cases() {
    assert_eq!(storage::seconds_to_ledgers(0), 0);
    assert_eq!(storage::seconds_to_ledgers(1), 1);
    assert_eq!(storage::seconds_to_ledgers(u64::MAX), u32::MAX);
}

#[test]
fn ttl_target_ledgers_already_expired() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 10 * DAY);

    // fast forward way past the end date
    h.warp_to(T0 + 100 * DAY);

    // Target should be floored at the minimum
    let target = storage::ttl_target_ledgers(&h.env, &h.get(id));
    assert_eq!(target, storage::MIN_STREAM_TTL_LEDGERS);
}
