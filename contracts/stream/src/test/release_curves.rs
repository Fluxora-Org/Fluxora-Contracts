//! Issue #1815 — release curves other than linear.
//!
//! `accrual::vested` used to be unconditionally `deposited * elapsed /
//! duration`. This module pins the replacement: a stream carries a
//! [`ReleaseCurve`], chosen at creation and immutable thereafter, and `vested`
//! follows it above the cliff.
//!
//! # What is asserted
//!
//! * **A non-linear curve can be created** — through the new
//!   `create_stream_with_curve` entry point — and is visible in `get_stream`
//!   *and* in the `StreamCreated` event, byte for byte.
//! * **Linear is the default and is unchanged.** `create_stream` still takes
//!   exactly the same arguments and still produces the same schedule; a linear
//!   stream created through the dedicated curve entry point is indistinguishable
//!   from one created through the original method, at every instant.
//! * **Every curve is monotone non-decreasing** in time, checked second by
//!   second across the whole schedule and through the contract's own views.
//! * **Total conservation holds for every curve**: `vested + refundable ==
//!   deposited` at every instant, the pool holds exactly the outstanding
//!   liability, and withdrawing then cancelling moves exactly the right tokens.
//! * **Storage is backwards-compatible.** The stored value stays the frozen v1
//!   [`StreamRecord`] and a stream written by a v1 deployment — a record with no
//!   curve side-car — reads back as [`ReleaseCurve::Linear`] and vests exactly
//!   as it did before. That is the property that lets a live deployment keep
//!   working without a migration.

use soroban_sdk::testutils::Events as _;
use soroban_sdk::{xdr, Event as _};

use super::common::*;
use crate::events::StreamCreated;
use crate::types::StreamRecord;
use crate::{accrual, CliffMode, DataKey, ReleaseCurve, StreamStatus};

/// Every curve under test. Adding a variant to [`ReleaseCurve`] and forgetting
/// this list fails `every_curve_is_covered_by_the_test_matrix`.
const ALL_CURVES: [ReleaseCurve; 3] = [
    ReleaseCurve::Linear,
    ReleaseCurve::Step,
    ReleaseCurve::FrontLoaded,
];

/// The stream contract's own events from the most recent invocation.
///
/// `Events::all()` only reports the last invocation, so this must be the first
/// thing a test does after the call under test — any other client call replaces
/// the snapshot. Filtering by the stream contract drops the SAC's `transfer`.
fn published_by_stream(h: &Harness) -> std::vec::Vec<xdr::ContractEvent> {
    h.env
        .events()
        .all()
        .filter_by_contract(&h.contract_id)
        .events()
        .to_vec()
}

/// Create a stream with an explicit curve through the new entry point.
#[allow(clippy::too_many_arguments)]
fn create_with_curve(
    h: &Harness,
    deposit: i128,
    duration: u64,
    cliff_offset: u64,
    curve: ReleaseCurve,
) -> u64 {
    let start = h.now();
    h.client.create_stream_with_curve(
        &h.sender,
        &h.recipient,
        &h.token,
        &deposit,
        &start,
        &(start + duration),
        &(start + cliff_offset),
        &true,
        &true,
        &true,
        &curve,
    )
}

/// The `StreamCreated` the contract must have emitted for `id` right now.
fn expected_created(h: &Harness, id: u64, curve: ReleaseCurve) -> xdr::ContractEvent {
    let s = h.get(id);
    StreamCreated {
        stream_id: id,
        sender: h.sender.clone(),
        recipient: h.recipient.clone(),
        token: h.token.clone(),
        deposited: s.deposited,
        start_time: s.start_time,
        end_time: s.end_time,
        cliff_time: s.cliff_time,
        cancellable: s.cancellable,
        pausable: s.pausable,
        transferable: s.transferable,
        curve,
        cliff_mode: s.cliff_mode,
        reference: s.reference.clone(),
    }
    .to_xdr(&h.env, &h.contract_id)
}

// ---------------------------------------------------------------------------
// The curve is created, stored, and observable
// ---------------------------------------------------------------------------

/// The matrix is complete: every [`ReleaseCurve`] variant is exercised by the
/// suites below. A new variant makes this fail rather than silently shipping
/// untested.
#[test]
fn every_curve_is_covered_by_the_test_matrix() {
    fn discriminant(c: ReleaseCurve) -> u32 {
        match c {
            ReleaseCurve::Linear => 0,
            ReleaseCurve::Step => 1,
            ReleaseCurve::FrontLoaded => 2,
        }
    }

    let seen: std::vec::Vec<u32> = ALL_CURVES.iter().copied().map(discriminant).collect();
    assert_eq!(seen, std::vec![0, 1, 2]);
}

/// `create_stream` still defaults to, and reports, [`ReleaseCurve::Linear`] —
/// both in `get_stream` and in the created event.
#[test]
fn create_stream_defaults_to_linear_and_reports_it() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);

    // Snapshot the events before any other client call replaces the buffer.
    let published = published_by_stream(&h);
    assert_eq!(
        published,
        std::vec![expected_created(&h, id, ReleaseCurve::Linear)],
        "create_stream must report Linear in the created event"
    );

    let s = h.get(id);
    assert_eq!(
        s.curve,
        ReleaseCurve::Linear,
        "create_stream must default to Linear"
    );
}

/// A non-linear curve can be created, and shows up in both `get_stream` and the
/// created event.
#[test]
fn create_stream_with_curve_sets_and_reports_the_curve() {
    for curve in [ReleaseCurve::Step, ReleaseCurve::FrontLoaded] {
        let h = Harness::new();
        let id = create_with_curve(&h, 1_000 * ONE, 100 * DAY, 0, curve);

        let published = published_by_stream(&h);
        assert_eq!(
            published,
            std::vec![expected_created(&h, id, curve)],
            "{curve:?}: the created event must carry the curve"
        );
        assert_eq!(
            h.get(id).curve,
            curve,
            "{curve:?}: get_stream must carry the curve"
        );
    }
}

/// The curve is part of the stored stream, so it survives every later
/// operation. A curve that could change — or be lost by a re-save — would make
/// the schedule a sender's promise rather than a guarantee.
#[test]
fn the_curve_survives_touch_operations_and_cancel() {
    let h = Harness::new();
    let id = create_with_curve(&h, 1_000 * ONE, 100 * DAY, 0, ReleaseCurve::Step);
    assert_eq!(h.get(id).curve, ReleaseCurve::Step);

    // Withdraw (a load → save round trip).
    h.advance(30 * DAY);
    h.client.withdraw(&id, &None);
    assert_eq!(
        h.get(id).curve,
        ReleaseCurve::Step,
        "withdraw lost the curve"
    );

    // Pause / resume (two more round trips).
    h.client.pause(&id);
    assert_eq!(h.get(id).curve, ReleaseCurve::Step, "pause lost the curve");
    h.client.resume(&id);
    h.assert_pool_exact();

    // Top up (a save path that also rewrites the record).
    let end_before = h.get(id).end_time;
    h.client.top_up(&id, &(100 * ONE));
    assert_eq!(h.get(id).curve, ReleaseCurve::Step, "top_up lost the curve");
    assert_eq!(
        h.get(id).end_time,
        end_before,
        "a non-linear top-up holds the schedule and scales the deposit"
    );

    // Cancel (the terminal rewrite).
    h.client.cancel(&id);
    assert_eq!(h.get(id).curve, ReleaseCurve::Step, "cancel lost the curve");
    h.assert_pool_exact();
}

// ---------------------------------------------------------------------------
// Linear behaviour is unchanged
// ---------------------------------------------------------------------------

/// The linear path is *the same arithmetic*, not a re-implementation:
/// `create_stream` and `create_stream_with_curve(Linear)` produce two streams
/// that vest identically at every instant.
#[test]
fn linear_via_the_curve_entry_point_is_identical_to_create_stream() {
    let h = Harness::new();
    let via_default = h.create_simple(1_000 * ONE, 100 * DAY);
    let via_curve = create_with_curve(&h, 1_000 * ONE, 100 * DAY, 0, ReleaseCurve::Linear);
    h.assert_pool_exact();

    let a = h.get(via_default);
    let b = h.get(via_curve);
    assert_eq!(a.curve, ReleaseCurve::Linear);
    assert_eq!(b.curve, ReleaseCurve::Linear);
    assert_eq!(a.deposited, b.deposited);

    for d in 0..=(100u64 * DAY + 10 * DAY) {
        let t = a.start_time + d;
        assert_eq!(
            accrual::vested(&a, t).unwrap(),
            accrual::vested(&b, t).unwrap(),
            "linear schedules diverged at t=+{d}s"
        );
    }
}

/// The original linear formula, at exact expected values, including the
/// rounding-down direction. This is the regression guard for "linear behaviour
/// is unchanged": if the curve dispatch touches the linear arm, these fail.
#[test]
fn linear_arithmetic_is_the_original_floor_formula() {
    let h = Harness::new();
    let id = create_with_curve(&h, 1_000, 1_000, 0, ReleaseCurve::Linear);
    let s = h.get(id);

    for (elapsed, expected) in [
        (0u64, 0i128),
        (1, 1),
        (333, 333),
        (999, 999),
        (1_000, 1_000),
    ] {
        assert_eq!(
            accrual::vested(&s, s.start_time + elapsed).unwrap(),
            expected,
            "linear vested at +{elapsed}s"
        );
    }
    // The floored tail: 1000 * 1 / 3 = 333, not 333.33.
    let s3 = h.get(create_with_curve(&h, 1_000, 3, 0, ReleaseCurve::Linear));
    assert_eq!(accrual::vested(&s3, s3.start_time + 1).unwrap(), 333);
    assert_eq!(accrual::vested(&s3, s3.start_time + 2).unwrap(), 666);
    assert_eq!(accrual::vested(&s3, s3.start_time + 3).unwrap(), 1_000);
}

// ---------------------------------------------------------------------------
// Monotonicity and conservation, per curve, across the full schedule
// ---------------------------------------------------------------------------

/// **The acceptance test.** For every curve: `vested` never decreases across
/// the whole schedule at one-second resolution, stays within `[0, deposited]`,
/// and `vested + refundable == deposited` at every one of those instants.
#[test]
fn every_curve_is_monotonic_and_conserving_second_by_second() {
    let h = Harness::new();
    let deposit = 1_000 * ONE;
    let duration = 100 * DAY;

    for curve in ALL_CURVES {
        let id = create_with_curve(&h, deposit, duration, 0, curve);
        let s = h.get(id);

        let mut prev = -1i128;
        assert_eq!(
            accrual::vested(&s, s.start_time).unwrap(),
            0,
            "{curve:?}: nothing vests at the start instant"
        );
        for d in 0..=(duration + DAY) {
            let t = s.start_time + d;
            let v = accrual::vested(&s, t).unwrap();
            assert!(
                v >= prev,
                "{curve:?}: vested went backwards at +{d}s: {v} < {prev}"
            );
            assert!(
                (0..=deposit).contains(&v),
                "{curve:?}: vested {v} out of [0, {deposit}] at +{d}s"
            );
            assert_eq!(
                v + accrual::refundable(&s, t).unwrap(),
                deposit,
                "{curve:?}: conservation failed at +{d}s"
            );
            prev = v;
        }
        assert_eq!(
            accrual::vested(&s, s.start_time + duration).unwrap(),
            deposit,
            "{curve:?}: the full schedule must vest the whole deposit"
        );
    }
}

/// The same properties through the *contract's* views (`vested_of`,
/// `refundable_of`, `withdrawable_of`), which is what an integrator actually
/// calls. The views read stored state, so this also proves the curve survives
/// the storage round trip.
#[test]
fn contract_views_are_monotonic_and_conserving_for_every_curve() {
    let deposit = 840 * ONE;
    let duration = 84 * DAY;

    for curve in ALL_CURVES {
        // Fresh harness per curve so the id space starts clean and the pool is
        // exactly this stream's liability.
        let h = Harness::new();
        let id = create_with_curve(&h, deposit, duration, 0, curve);

        let mut prev = -1i128;
        for day in 0..=duration / DAY + 1 {
            let v = h.client.vested_of(&id);
            assert!(v >= prev, "{curve:?}: view went backwards on day {day}");
            assert_eq!(
                v + h.client.refundable_of(&id),
                deposit,
                "{curve:?}: view conservation failed on day {day}"
            );
            assert_eq!(
                h.client.withdrawable_of(&id),
                v - h.get(id).withdrawn,
                "{curve:?}: withdrawable disagrees with vested - withdrawn on day {day}"
            );
            h.assert_invariants();
            prev = v;
            h.advance(DAY);
        }
        assert_eq!(prev, deposit, "{curve:?}: views must settle at the deposit");
    }
}

/// The curve shapes are genuinely distinct, at exact values. If `Step` and
/// `FrontLoaded` both collapsed to the linear schedule the feature would be
/// vacuous and the "monotone" assertions above would be trivially true.
#[test]
fn curve_shapes_are_distinct_at_exact_instants() {
    let h = Harness::new();
    // A quarter of 1000 is 250, so the tranches land on exact values.
    let (deposit, duration) = (1_000 * ONE, 100 * DAY);
    let quarter = duration / 4;

    let linear = h.get(create_with_curve(
        &h,
        deposit,
        duration,
        0,
        ReleaseCurve::Linear,
    ));
    let step = h.get(create_with_curve(
        &h,
        deposit,
        duration,
        0,
        ReleaseCurve::Step,
    ));
    let front = h.get(create_with_curve(
        &h,
        deposit,
        duration,
        0,
        ReleaseCurve::FrontLoaded,
    ));

    let vested_at =
        |s: &crate::Stream, offset: u64| accrual::vested(s, s.start_time + offset).unwrap();

    // One second before the first quarter: linear has accrued, Step has not.
    let before = quarter - 1;
    assert_eq!(
        vested_at(&step, before),
        0,
        "Step must not accrue before its tranche"
    );
    assert!(vested_at(&linear, before) > 0, "linear must have accrued");

    // At the first quarter: linear and Step agree at exactly a quarter, and
    // FrontLoaded leads both.
    assert_eq!(vested_at(&step, quarter), deposit / 4);
    assert_eq!(vested_at(&linear, quarter), deposit / 4);
    assert!(
        vested_at(&front, quarter) > deposit / 4,
        "FrontLoaded must lead the others inside the schedule"
    );

    // Step is piecewise: flat, then a jump at the boundary.
    assert!(vested_at(&step, quarter) > vested_at(&step, before));
    assert_eq!(vested_at(&step, quarter + 1), vested_at(&step, quarter));
    assert_eq!(vested_at(&step, 2 * quarter), deposit / 2);
    assert_eq!(vested_at(&step, 3 * quarter), 3 * deposit / 4);
    assert_eq!(vested_at(&step, duration), deposit);

    // FrontLoaded never trails linear at the same *grid point* (its defining
    // property is `2u - u² >= u` on `[0, 1]`; the 1/1000 evaluation grid is
    // what makes the comparison exact), and it settles at the same endpoint.
    let step = duration / 1_000;
    for k in [1u64, 250, 500, 750, 999] {
        let offset = k * step;
        assert!(
            vested_at(&front, offset) >= vested_at(&linear, offset),
            "FrontLoaded must never trail linear at u={k}/1000"
        );
    }
    assert_eq!(vested_at(&front, duration), deposit);
}

/// The cliff gates every curve identically: nothing before it, and everything
/// accrued since the *start* (not since the cliff) at the cliff instant.
#[test]
fn the_cliff_gates_every_curve() {
    let h = Harness::new();
    let (deposit, duration) = (1_000 * ONE, 100 * DAY);
    let cliff_offset = 50 * DAY;

    for curve in ALL_CURVES {
        let id = create_with_curve(&h, deposit, duration, cliff_offset, curve);
        let s = h.get(id);

        assert_eq!(
            accrual::vested(&s, s.start_time + cliff_offset - 1).unwrap(),
            0,
            "{curve:?}: nothing may vest before the cliff"
        );
        let at_cliff = accrual::vested(&s, s.start_time + cliff_offset).unwrap();
        // Whatever the curve, the cliff release equals the curve evaluated at
        // the cliff offset — accrual is *gated*, not restarted.
        let mut past_cliff = s.clone();
        past_cliff.cliff_time = s.start_time;
        assert_eq!(
            at_cliff,
            accrual::vested(&past_cliff, s.start_time + cliff_offset).unwrap(),
            "{curve:?}: the cliff must gate, not delay"
        );
    }
}

/// Pausing freezes every curve's accrual, and resuming continues it from where
/// it stopped. A curve must not leak progress while the clock is frozen.
#[test]
fn pausing_freezes_every_curve() {
    let (deposit, duration) = (1_000 * ONE, 100 * DAY);

    for curve in ALL_CURVES {
        let h = Harness::new();
        let id = create_with_curve(&h, deposit, duration, 0, curve);

        h.advance(30 * DAY);
        h.client.pause(&id);
        let frozen = h.client.vested_of(&id);

        h.advance(20 * DAY);
        assert_eq!(
            h.client.vested_of(&id),
            frozen,
            "{curve:?}: accrual must be frozen while paused"
        );

        h.client.resume(&id);
        assert_eq!(
            h.client.vested_of(&id),
            frozen,
            "{curve:?}: resume must not retroactively accrue the paused window"
        );

        h.advance(10 * DAY);
        assert!(
            h.client.vested_of(&id) >= frozen,
            "{curve:?}: accrual must resume from the frozen point"
        );
        h.assert_pool_exact();
    }
}

// ---------------------------------------------------------------------------
// Withdraw and cancel move the curve's numbers
// ---------------------------------------------------------------------------

/// Withdrawal pays out exactly what the curve has released, and cancellation
/// refunds exactly what it has not — with no dust created or stranded, for
/// every curve.
#[test]
fn withdraw_and_cancel_follow_the_curve_exactly() {
    let deposit = 1_000 * ONE;
    let duration = 100 * DAY;

    for curve in ALL_CURVES {
        let h = Harness::new();
        let id = create_with_curve(&h, deposit, duration, 0, curve);

        h.advance(30 * DAY);
        let vested = h.client.vested_of(&id);
        assert!(
            vested > 0,
            "{curve:?}: something must have vested by day 30"
        );

        let recipient_before = h.balance(&h.recipient);
        assert_eq!(h.client.withdraw(&id, &None), vested);
        assert_eq!(h.balance(&h.recipient) - recipient_before, vested);
        assert_eq!(h.client.withdrawable_of(&id), 0);
        h.assert_pool_exact();

        // Advance into a later part of the schedule and cancel.
        h.advance(20 * DAY);
        let vested_at_cancel = h.client.vested_of(&id);
        assert!(vested_at_cancel >= vested, "{curve:?}: vested regressed");

        let sender_before = h.balance(&h.sender);
        h.client.cancel(&id);
        let refunded = h.balance(&h.sender) - sender_before;

        assert_eq!(
            refunded + vested_at_cancel,
            deposit,
            "{curve:?}: refund + vested must partition the deposit exactly"
        );
        let s = h.get(id);
        assert_eq!(s.status, StreamStatus::Cancelled);
        assert_eq!(
            s.deposited, vested_at_cancel,
            "post-cancel deposited is the vested total"
        );
        assert_eq!(s.curve, curve, "{curve:?}: cancel must preserve the curve");
        // What was already withdrawn stays withdrawn; the remainder is still
        // claimable and is exactly what the pool holds.
        assert_eq!(h.client.withdrawable_of(&id), vested_at_cancel - vested);
        assert_eq!(h.pool(), vested_at_cancel - vested);
        h.assert_pool_exact();

        // The recipient can still drain the remainder.
        h.client.withdraw(&id, &None);
        assert_eq!(h.pool(), 0);
    }
}

/// A top-up on a non-linear stream holds the schedule and scales the deposit,
/// and can never reduce what the recipient has already earned.
#[test]
fn top_up_on_a_non_linear_stream_never_reduces_vested() {
    let h = Harness::new();
    let id = create_with_curve(&h, 1_000 * ONE, 100 * DAY, 0, ReleaseCurve::Step);

    h.advance(30 * DAY);
    let vested_before = h.client.vested_of(&id);
    let end_before = h.get(id).end_time;
    let deposited_before = h.get(id).deposited;

    h.client.top_up(&id, &(400 * ONE));

    let s = h.get(id);
    assert_eq!(s.deposited, deposited_before + 400 * ONE);
    assert_eq!(
        s.end_time, end_before,
        "a non-linear top-up must hold the schedule"
    );
    assert!(
        h.client.vested_of(&id) >= vested_before,
        "top_up must never reduce vested"
    );
    assert_eq!(s.curve, ReleaseCurve::Step);
    h.assert_pool_exact();
}

// ---------------------------------------------------------------------------
// Storage backwards compatibility
// ---------------------------------------------------------------------------

/// A stream written by a *v1* deployment — the frozen [`StreamRecord`] under
/// `DataKey::Stream(id)`, with no curve side-car — reads back as
/// [`ReleaseCurve::Linear`] and vests exactly as it did before curves existed.
///
/// This is the migration story: nothing has to be rewritten for a live
/// deployment to keep working. A v1 entry is not upgraded on read; the missing
/// side-car simply means "linear", which is what it was created as.
#[test]
fn a_v1_record_with_no_side_car_reads_back_as_linear() {
    let h = Harness::new();
    let start = h.now();
    let duration = 100 * DAY;
    let deposit = 1_000 * ONE;

    // Write the v1 shape directly, exactly as a pre-#1815 deployment would
    // have: only `DataKey::Stream(id)`, holding a `StreamRecord`.
    let record = StreamRecord {
        sender: h.sender.clone(),
        recipient: h.recipient.clone(),
        token: h.token.clone(),
        deposited: deposit,
        withdrawn: 0,
        start_time: start,
        end_time: start + duration,
        cliff_time: start,
        cliff_mode: CliffMode::DEFAULT,
        cancellable: true,
        pausable: true,
        transferable: true,
        paused_at: None,
        paused_total: 0,
        status: StreamStatus::Active,
        reference: None,
    };
    let id = 7u64;
    h.env.as_contract(&h.contract_id, || {
        h.env
            .storage()
            .persistent()
            .set(&DataKey::Stream(id), &record);
        assert!(
            !h.env.storage().persistent().has(&DataKey::StreamCurve(id)),
            "a v1 entry must have no curve side-car"
        );
    });

    // The current reader decodes it and applies the linear default.
    let s = h.client.get_stream(&id);
    assert_eq!(
        s.curve,
        ReleaseCurve::Linear,
        "a missing side-car means Linear"
    );
    assert_eq!(s.deposited, deposit);
    assert_eq!(
        h.client.vested_of(&id),
        0,
        "nothing vests at the start instant"
    );

    h.advance(25 * DAY);
    assert_eq!(
        h.client.vested_of(&id),
        deposit / 4,
        "a v1 entry must vest on the original linear schedule"
    );
    assert_eq!(h.client.refundable_of(&id), deposit - deposit / 4);
}

/// The stored value really is the frozen [`StreamRecord`], not a `Stream` with
/// the new field — and the side-car appears only for non-linear curves, so a
/// linear stream pays no rent for a key that would merely restate the default.
#[test]
fn storage_holds_the_frozen_record_and_only_non_linear_curves_get_a_side_car() {
    let h = Harness::new();
    let id = create_with_curve(&h, 1_000 * ONE, 100 * DAY, 0, ReleaseCurve::FrontLoaded);

    h.env.as_contract(&h.contract_id, || {
        // Decodes as the frozen record — a `Stream` value would not.
        let record: StreamRecord = h
            .env
            .storage()
            .persistent()
            .get(&DataKey::Stream(id))
            .expect("the stored entry must decode as the frozen v1 StreamRecord");
        assert_eq!(record.deposited, 1_000 * ONE);

        let curve: ReleaseCurve = h
            .env
            .storage()
            .persistent()
            .get(&DataKey::StreamCurve(id))
            .expect("a non-linear stream must have a curve side-car");
        assert_eq!(curve, ReleaseCurve::FrontLoaded);
    });

    // A linear stream, in contrast, writes no side-car at all.
    let linear_id = create_with_curve(&h, 1_000 * ONE, 100 * DAY, 0, ReleaseCurve::Linear);
    h.env.as_contract(&h.contract_id, || {
        assert!(
            !h.env
                .storage()
                .persistent()
                .has(&DataKey::StreamCurve(linear_id)),
            "a linear stream must not pay for a side-car key"
        );
    });

    // And the side-car is kept alive alongside the record it annotates.
    let side_car_ttl = h.env.as_contract(&h.contract_id, || {
        use soroban_sdk::testutils::storage::Persistent as _;
        h.env
            .storage()
            .persistent()
            .get_ttl(&DataKey::StreamCurve(id))
    });
    assert!(side_car_ttl > 0, "the curve side-car must have a live TTL");
    assert_eq!(
        h.ttl_of(id),
        side_car_ttl,
        "side-car and record share a TTL target"
    );
}
