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

use proptest::prelude::*;
use soroban_sdk::testutils::storage::Persistent as _;
use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::Address;

use super::common::*;
use crate::{storage, DataKey, Error, StreamStatus, TTL_BUFFER_SECONDS};

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

    age_ledgers(&h, ttl_of(&h, id) - 1_000);
    h.client.transfer_recipient(&id, &Address::generate(&h.env));
    assert!(
        ttl_of(&h, id) > 1_000_000,
        "transfer_recipient did not re-extend"
    );

    age_ledgers(&h, ttl_of(&h, id) - 1_000);
    h.client.cancel(&id);
    assert!(ttl_of(&h, id) > 1_000_000, "cancel did not re-extend");
}

/// After any touch an active stream is funded for exactly its remaining life
/// plus the buffer — no more, no less. Threshold equals extend-to, so the
/// equality is exact, not a lower bound.
#[test]
fn a_touched_stream_is_funded_for_exactly_its_remaining_life_plus_buffer() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);

    h.advance(30 * DAY);
    let target = h.client.extend_stream_ttl(&id);

    let expected = storage::seconds_to_ledgers(70 * DAY + TTL_BUFFER_SECONDS);
    assert_eq!(target, expected);
    assert_eq!(ttl_of(&h, id), expected);
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

// --- Retention by state ----------------------------------------------------

/// Cancelling collapses the schedule onto "now", so the rent target drops to
/// the settled floor. Rent already paid is never clawed back, though: the
/// entry keeps the horizon its last active touch funded and decays toward the
/// floor, which is where later sweeps hold it.
#[test]
fn a_cancelled_stream_settles_to_the_floor_and_stays_readable() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);

    h.advance(40 * DAY);
    h.client.cancel(&id);
    assert_eq!(h.get(id).status, StreamStatus::Cancelled);

    // The cancel touched the entry while it still had 60 days to run, and
    // that funding stands: cancellation must never shorten a TTL.
    let funded_at_cancel = storage::seconds_to_ledgers(60 * DAY + TTL_BUFFER_SECONDS);
    assert_eq!(ttl_of(&h, id), funded_at_cancel);

    // A sweep now targets only the floor, so the higher balance is left alone.
    h.advance(10 * DAY);
    assert_eq!(
        h.client.extend_stream_ttl(&id),
        storage::MIN_STREAM_TTL_LEDGERS
    );
    assert_eq!(
        ttl_of(&h, id),
        funded_at_cancel - storage::seconds_to_ledgers(10 * DAY)
    );

    // Once the rent decays below the floor, sweeps hold it exactly there.
    age_ledgers(&h, ttl_of(&h, id) - 1_000);
    h.client.extend_stream_ttl(&id);
    assert_eq!(ttl_of(&h, id), storage::MIN_STREAM_TTL_LEDGERS);

    // And the tail the recipient earned is still there to withdraw.
    assert_eq!(h.client.withdraw(&id, &None), 400 * ONE);
    h.assert_pool_exact();
}

/// A drained stream sits exactly on the floor: by its end the creation-time
/// rent has decayed to precisely the buffer, and every later touch re-targets
/// that same floor. Fully paid out is not the same as forgotten.
#[test]
fn a_depleted_stream_settles_to_the_floor() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 10 * DAY);

    h.warp_to(T0 + 10 * DAY);
    assert_eq!(h.client.withdraw(&id, &None), 1_000 * ONE);
    assert_eq!(h.get(id).status, StreamStatus::Depleted);

    assert_eq!(ttl_of(&h, id), storage::MIN_STREAM_TTL_LEDGERS);
    assert_eq!(
        h.client.extend_stream_ttl(&id),
        storage::MIN_STREAM_TTL_LEDGERS
    );
}

// --- Missing entries -------------------------------------------------------

/// Every single-stream entry point answers a never-issued id with
/// `StreamNotFound`, and a call that fails this way must not move a thing:
/// not the counter, not a live stream's record or rent, not the pool.
/// `batch_withdraw` propagates the same error for its whole batch, while
/// `batch_extend_ttl` deliberately skips instead (pinned further up).
///
/// This is also the contract-side half of the archival story: on a real
/// network a transaction touching an archived entry fails *before* the
/// contract runs and needs a `RestoreFootprint` (KNOWN-LIMITATIONS §1). The
/// contract itself only ever says `StreamNotFound`, for ids it cannot see.
#[test]
fn a_missing_stream_returns_stream_not_found_and_mutates_nothing() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    let live = h.get(id);
    let rent = ttl_of(&h, id);
    let pool = h.pool();
    let missing = 999_u64;

    assert!(!h.client.stream_exists(&missing));
    assert_eq!(
        h.client.try_get_stream(&missing).unwrap_err().unwrap(),
        Error::StreamNotFound
    );
    assert_eq!(
        h.client.try_withdrawable_of(&missing).unwrap_err().unwrap(),
        Error::StreamNotFound
    );
    assert_eq!(
        h.client.try_vested_of(&missing).unwrap_err().unwrap(),
        Error::StreamNotFound
    );
    assert_eq!(
        h.client.try_refundable_of(&missing).unwrap_err().unwrap(),
        Error::StreamNotFound
    );
    assert_eq!(
        h.client.try_withdraw(&missing, &None).unwrap_err().unwrap(),
        Error::StreamNotFound
    );
    assert_eq!(
        h.client
            .try_top_up(&missing, &(10 * ONE))
            .unwrap_err()
            .unwrap(),
        Error::StreamNotFound
    );
    assert_eq!(
        h.client.try_pause(&missing).unwrap_err().unwrap(),
        Error::StreamNotFound
    );
    assert_eq!(
        h.client.try_resume(&missing).unwrap_err().unwrap(),
        Error::StreamNotFound
    );
    assert_eq!(
        h.client.try_cancel(&missing).unwrap_err().unwrap(),
        Error::StreamNotFound
    );
    assert_eq!(
        h.client
            .try_transfer_recipient(&missing, &Address::generate(&h.env))
            .unwrap_err()
            .unwrap(),
        Error::StreamNotFound
    );
    assert_eq!(
        h.client
            .try_extend_stream_ttl(&missing)
            .unwrap_err()
            .unwrap(),
        Error::StreamNotFound
    );
    assert_eq!(
        h.client
            .try_batch_withdraw(&h.recipient, &h.ids(&[missing]))
            .unwrap_err()
            .unwrap(),
        Error::StreamNotFound
    );

    assert_eq!(h.client.stream_count(), 1, "counter must not move");
    assert_eq!(h.get(id), live, "the live stream's record must not change");
    assert_eq!(
        ttl_of(&h, id),
        rent,
        "the live stream's rent must not change"
    );
    assert_eq!(h.pool(), pool);
    h.assert_pool_exact();
}

// --- Unit coverage of the rent arithmetic ----------------------------------

#[test]
fn seconds_to_ledgers_rounds_up() {
    assert_eq!(storage::seconds_to_ledgers(0), 0);
    assert_eq!(
        storage::seconds_to_ledgers(1),
        1,
        "a partial ledger still counts"
    );
    assert_eq!(storage::seconds_to_ledgers(storage::SECONDS_PER_LEDGER), 1);
    assert_eq!(
        storage::seconds_to_ledgers(storage::SECONDS_PER_LEDGER + 1),
        2
    );
    assert_eq!(storage::seconds_to_ledgers(DAY), 17_280);
    // Saturates rather than wrapping.
    assert_eq!(storage::seconds_to_ledgers(u64::MAX), u32::MAX);
}

/// A duration one second shy of a full ledger must still buy the whole
/// ledger, not zero — the same "any partial ledger counts" rule as `1`, just
/// approached from the other side of the boundary.
#[test]
fn seconds_to_ledgers_rounds_up_just_below_one_ledger() {
    assert_eq!(
        storage::seconds_to_ledgers(storage::SECONDS_PER_LEDGER - 1),
        1,
    );
}

/// Exact multiples of the ledger length must convert without residue, at
/// several scales — not just the single-ledger and one-day cases above.
#[test]
fn seconds_to_ledgers_exact_multiples() {
    for n in [2u64, 3, 10, 100, 1_000, 100_000] {
        let seconds = n * storage::SECONDS_PER_LEDGER;
        assert_eq!(storage::seconds_to_ledgers(seconds), n as u32, "n = {n}",);
    }
}

/// The largest input that converts without hitting the `u32::MAX` saturation
/// clamp, and the smallest one that does. Saturation is intentional (see the
/// doc comment on `seconds_to_ledgers`), but the clamp must engage exactly one
/// second past the true boundary — not early, and not late.
#[test]
fn seconds_to_ledgers_saturation_boundary_is_exact() {
    let last_exact = u32::MAX as u64 * storage::SECONDS_PER_LEDGER;
    assert_eq!(
        storage::seconds_to_ledgers(last_exact),
        u32::MAX,
        "the true boundary value must convert exactly, not saturate early"
    );
    assert_eq!(
        storage::seconds_to_ledgers(last_exact + 1),
        u32::MAX,
        "one second past the boundary must saturate, not overflow"
    );
}

// --- Property coverage of the rounding guarantee ---------------------------

proptest! {
    #![proptest_config(ProptestConfig::default())]

    /// **The property ceiling rounding exists to guarantee.**
    ///
    /// Converting seconds to ledgers and back can never promise *less*
    /// wall-clock time than was asked for — that is the entire reason the
    /// design chose ceiling over floor (see the doc comment on
    /// `seconds_to_ledgers`). Bounded to the pre-saturation domain: past
    /// `u32::MAX` ledgers the function's contract deliberately switches to
    /// saturation, which is covered separately by
    /// `seconds_to_ledgers_saturation_boundary_is_exact` and the `u64::MAX`
    /// case in `seconds_to_ledgers_rounds_up`.
    #[test]
    fn seconds_to_ledgers_round_trip_never_undershoots(
        seconds in 0u64..=(u32::MAX as u64 * storage::SECONDS_PER_LEDGER)
    ) {
        let ledgers = storage::seconds_to_ledgers(seconds);
        let recovered = ledgers as u64 * storage::SECONDS_PER_LEDGER;

        prop_assert!(
            recovered >= seconds,
            "round trip undershot: {} seconds -> {} ledgers -> {} seconds",
            seconds, ledgers, recovered,
        );
        // Ceiling, not some looser bound: never overshoots by more than one
        // ledger's worth either.
        prop_assert!(
            recovered - seconds < storage::SECONDS_PER_LEDGER,
            "overshot by more than one ledger's worth: {} -> {}",
            seconds, recovered,
        );
    }
}

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
