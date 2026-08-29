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

use soroban_sdk::testutils::storage::Persistent as _;
use soroban_sdk::testutils::Ledger as _;

use super::common::*;
use crate::{storage, DataKey, TTL_BUFFER_SECONDS};

#[test]
fn persisted_stream_fixture_survives_read_mutate_and_ttl_extension() {
    let h = super::common::Harness::new();
    let id = h.create_simple(1_000 * super::common::ONE, 100 * super::common::DAY);
    let before = h.get(id);

    h.client.top_up(&id, &(250 * super::common::ONE));
    let after = h.get(id);

    assert_eq!(after.sender, before.sender);
    assert_eq!(after.recipient, before.recipient);
    assert_eq!(after.token, before.token);
    assert_eq!(after.withdrawn, before.withdrawn);
    assert_eq!(after.deposited, 1_250 * super::common::ONE);
    assert!(ttl_of(&h, id) > 0);
}

/// Remaining TTL, in ledgers, of a stream entry.
fn ttl_of(h: &Harness, stream_id: u64) -> u32 {
    h.env.as_contract(&h.contract_id, || {
        h.env
            .storage()
            .persistent()
            .get_ttl(&DataKey::Stream(stream_id))
    })
}

/// True if the entry shows the signature of a host auto-restore: a TTL pinned
/// to the network minimum, which this contract never sets deliberately.
fn was_restored(h: &Harness, stream_id: u64) -> bool {
    let min = h.env.ledger().get().min_persistent_entry_ttl;
    ttl_of(h, stream_id) < min
}

/// The largest TTL any entry can actually hold right now.
///
/// This is deliberately read from the SDK rather than from
/// `LedgerInfo::max_entry_ttl`: the achievable maximum is
/// `max_live_until_ledger - sequence`, which is not always the raw configured
/// value. Asserting against the config number bakes in an off-by-one.
fn max_achievable_ttl(h: &Harness) -> u32 {
    h.env
        .as_contract(&h.contract_id, || h.env.storage().max_ttl())
}

/// Advance only the ledger sequence, leaving the clock alone. Used to age
/// entries without moving accrual.
fn age_ledgers(h: &Harness, ledgers: u32) {
    let seq = h.env.ledger().sequence();
    h.env.ledger().set_sequence_number(seq + ledgers);
}

// --- Extension at creation -------------------------------------------------

/// A new stream must be funded with rent covering its whole scheduled life
/// plus the keeper's working buffer, so an ordinary stream never needs a
/// keeper at all.
#[test]
fn creation_covers_the_whole_stream_plus_the_buffer() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);

    let expected = storage::seconds_to_ledgers(100 * DAY + TTL_BUFFER_SECONDS);
    assert_eq!(ttl_of(&h, id), expected);

    // Sanity: that is meaningfully longer than the network's default minimum.
    let min = h.env.ledger().get().min_persistent_entry_ttl;
    assert!(
        expected > min * 100,
        "creation TTL barely above the default"
    );
}

/// A multi-year stream exceeds `max_entry_ttl`, so it clamps — which is exactly
/// why the permissionless keeper path has to exist.
#[test]
fn a_long_stream_clamps_to_the_network_maximum() {
    let h = Harness::new();
    let max = max_achievable_ttl(&h);
    let id = h.create_simple(10_000 * ONE, 5 * YEAR);

    assert_eq!(ttl_of(&h, id), max, "must clamp, never exceed");
    assert!(
        storage::seconds_to_ledgers(5 * YEAR) > max,
        "this test is only meaningful if the stream outlives the max TTL",
    );
}

/// A settled stream still has to stay readable: the recipient may not have
/// pulled their tail, and the indexer needs the final state.
#[test]
fn a_matured_stream_keeps_a_floor_of_rent() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 10 * DAY);
    h.warp_to(T0 + 100 * DAY);

    h.client.extend_stream_ttl(&id);
    assert_eq!(ttl_of(&h, id), storage::MIN_STREAM_TTL_LEDGERS);
}

/// A paused stream's end date slides forward in wall-clock terms, so its rent
/// target has to slide with it.
#[test]
fn a_paused_stream_is_funded_for_its_stretched_end() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);

    h.advance(10 * DAY);
    h.client.pause(&id);
    h.advance(200 * DAY);

    // An unpaused stream would be 110 days past its end by now and would sit on
    // the bare floor. This one is still 90 days from delivering, so it must be
    // funded for those 90 days plus the buffer.
    let target = h.client.extend_stream_ttl(&id);
    let expected = storage::seconds_to_ledgers(90 * DAY + TTL_BUFFER_SECONDS);
    assert_eq!(target, expected);
    assert!(
        target > storage::MIN_STREAM_TTL_LEDGERS,
        "a paused stream must not be treated as already settled",
    );
}

// --- Extension on every touch ----------------------------------------------

/// An actively-used stream never expires, because every mutating call tops its
/// rent back up.
#[test]
fn every_mutating_call_re_extends_the_ttl() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let full = ttl_of(&h, id);

    // Let most of the rent burn off, then touch the stream.
    age_ledgers(&h, full - 1_000);
    assert!(ttl_of(&h, id) < 2_000, "TTL should have decayed");

    h.advance(10 * DAY);
    h.client.withdraw(&id, &None);
    assert!(
        ttl_of(&h, id) > full - 200_000,
        "withdraw did not re-extend"
    );

    age_ledgers(&h, ttl_of(&h, id) - 1_000);
    h.client.pause(&id);
    assert!(ttl_of(&h, id) > 1_000_000, "pause did not re-extend");

    age_ledgers(&h, ttl_of(&h, id) - 1_000);
    h.client.resume(&id);
    assert!(ttl_of(&h, id) > 1_000_000, "resume did not re-extend");

    age_ledgers(&h, ttl_of(&h, id) - 1_000);
    h.client.top_up(&id, &(10 * ONE));
    assert!(ttl_of(&h, id) > 1_000_000, "top_up did not re-extend");
}

/// **Deliverable: a stream outlives the default TTL via the keeper path.**
///
/// The network is configured here so a single extension cannot cover the
/// stream's life — the situation every multi-year payroll or vesting stream is
/// actually in. A keeper sweeps periodically, and the stream survives a full
/// year with its accounting intact and pays out in full at the end.
#[test]
fn a_year_long_stream_survives_on_keeper_sweeps_alone() {
    let h = Harness::new();

    // Force the clamp: max rent buys ~5.8 days, but the stream runs a year.
    const MAX_TTL: u32 = 100_000;
    h.env.ledger().set_max_entry_ttl(MAX_TTL);

    let id = h.create_simple(365 * ONE, YEAR);
    assert_eq!(ttl_of(&h, id), MAX_TTL, "creation clamped as expected");

    // Nobody touches the stream all year except the keeper, sweeping at 60% of
    // the rent window — the cadence the backend keeper would actually use.
    let sweep_every = MAX_TTL * 6 / 10;
    let mut sweeps = 0;
    let mut lowest_seen = MAX_TTL;

    while h.now() < T0 + YEAR {
        h.advance(sweep_every as u64 * storage::SECONDS_PER_LEDGER);

        let before_sweep = ttl_of(&h, id);
        lowest_seen = lowest_seen.min(before_sweep);
        assert!(
            !was_restored(&h, id),
            "stream archived between sweeps after {sweeps} sweeps",
        );

        h.client.extend_stream_ttl(&id);
        assert_eq!(ttl_of(&h, id), MAX_TTL, "sweep did not restore full rent");
        sweeps += 1;
    }

    assert!(
        sweeps > 100,
        "expected many sweeps over a year, got {sweeps}"
    );
    assert!(lowest_seen < MAX_TTL / 2, "rent never actually decayed");

    // A full year later the accounting is untouched and the money is all there.
    let s = h.get(id);
    assert_eq!(s.deposited, 365 * ONE);
    assert_eq!(s.withdrawn, 0);
    assert_eq!(h.client.vested_of(&id), 365 * ONE);
    assert_eq!(h.client.withdraw(&id, &None), 365 * ONE);
    assert_eq!(h.balance(&h.recipient), 365 * ONE);
    h.assert_pool_exact();
}

/// The keeper is not privileged. Anyone — the recipient, a third party, a bot
/// with no relationship to either party — can pay to keep a claim readable.
#[test]
fn any_third_party_can_keep_a_stream_alive() {
    let h = Harness::new();
    h.env.ledger().set_max_entry_ttl(50_000);
    let id = h.create_simple(1_000 * ONE, YEAR);

    age_ledgers(&h, 40_000);
    let decayed = ttl_of(&h, id);
    assert!(decayed < 15_000);

    // No auth context at all, and no relationship to the stream.
    h.env.mock_auths(&[]);
    h.client.extend_stream_ttl(&id);

    assert_eq!(ttl_of(&h, id), 50_000);
}

/// **Deliverable: an archived stream restores with balances intact.**
///
/// The entry is left to archive with no keeper, then read. The host restores it
/// exactly as a `RestoreFootprint` would, and every field of the accounting —
/// deposit, withdrawals, schedule, status — must come back unchanged, with the
/// pooled tokens still fully backing it.
#[test]
fn an_archived_stream_restores_with_its_accounting_intact() {
    let h = Harness::new();
    h.env.ledger().set_max_entry_ttl(20_000);

    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(30 * DAY);
    h.client.withdraw(&id, &Some(100 * ONE));
    let before = h.get(id);
    let pool_before = h.pool();

    // Nobody sweeps. Let the rent run out completely.
    age_ledgers(&h, 100_000);

    // The tokens never moved — they sit in the contract's pooled balance
    // whatever happens to the accounting entry.
    assert_eq!(
        h.pool(),
        pool_before,
        "pooled funds are not affected by TTL"
    );

    // Reading restores the entry.
    let after = h.get(id);
    assert!(
        was_restored(&h, id),
        "entry should have gone through a restore"
    );

    assert_eq!(after, before, "restored stream differs from the original");
    assert_eq!(after.deposited, 1_000 * ONE);
    assert_eq!(after.withdrawn, 100 * ONE);

    // And it is fully functional again: the remaining claim pays out correctly.
    h.warp_to(T0 + 100 * DAY);
    assert_eq!(h.client.withdraw(&id, &None), 900 * ONE);
    assert_eq!(h.balance(&h.recipient), 1_000 * ONE);
    h.assert_pool_exact();
}

/// A restored entry must not be left on minimum rent — the next touch has to
/// re-fund it, or it would archive again almost immediately.
#[test]
fn a_restored_stream_is_re_funded_on_the_next_touch() {
    let h = Harness::new();
    h.env.ledger().set_max_entry_ttl(20_000);
    let id = h.create_simple(1_000 * ONE, 100 * DAY);

    age_ledgers(&h, 100_000);
    assert!(was_restored(&h, id));

    h.client.extend_stream_ttl(&id);
    assert_eq!(
        ttl_of(&h, id),
        20_000,
        "restore must be followed by re-funding"
    );
    assert!(!was_restored(&h, id));
}

/// A keeper working from a slightly stale index must not lose a whole sweep to
/// one bad id.
#[test]
fn batch_extend_skips_unknown_ids_without_failing() {
    let h = Harness::new();
    h.env.ledger().set_max_entry_ttl(50_000);
    let a = h.create_simple(100 * ONE, YEAR);
    let b = h.create_simple(100 * ONE, YEAR);

    age_ledgers(&h, 40_000);
    let extended = h.client.batch_extend_ttl(&h.ids(&[a, 999, b, 1_000]));

    assert_eq!(extended, 2, "should extend the two real streams");
    assert_eq!(ttl_of(&h, a), 50_000);
    assert_eq!(ttl_of(&h, b), 50_000);
}

/// The instance entry carries the id counter. If it archived, `create_stream`
/// would restart ids from zero and collide with live streams.
#[test]
fn the_instance_entry_is_kept_at_maximum_rent() {
    use soroban_sdk::testutils::storage::Instance as _;

    let h = Harness::new();
    let max = max_achievable_ttl(&h);
    h.create_simple(1_000 * ONE, 100 * DAY);

    let instance_ttl = h
        .env
        .as_contract(&h.contract_id, || h.env.storage().instance().get_ttl());
    assert_eq!(instance_ttl, max);
}

/// Ids stay unique across an archive/restore of the instance entry.
#[test]
fn stream_ids_never_collide_after_a_restore() {
    let h = Harness::new();
    let first = h.create_simple(100 * ONE, 100 * DAY);

    age_ledgers(&h, h.env.ledger().get().max_entry_ttl + 50_000);

    let second = h.create_simple(100 * ONE, 100 * DAY);
    assert_ne!(first, second);
    assert_eq!(second, 1);
    assert_eq!(h.client.stream_count(), 2);
}

// --- Unit coverage of the rent arithmetic ----------------------------------

#[test]
fn seconds_to_ledgers_rounds_up() {
    assert_eq!(storage::seconds_to_ledgers(0), 0);
    assert_eq!(storage::seconds_to_ledgers(1), 1);
    assert_eq!(storage::seconds_to_ledgers(u64::MAX), u32::MAX);
}
