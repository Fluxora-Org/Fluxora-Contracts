//! The `MAX_BATCH_SIZE` ceiling, enforced identically by every batch entry point (#1866).
//!
//! [`crate::MAX_BATCH_SIZE`] is declared once and consumed by two entry points:
//! `batch_withdraw` and `batch_extend_ttl`. `test::batch` already covers the
//! interesting *behaviour* of each — what a batch pays, when it rolls back,
//! which ids it tolerates — and touches the cap from both sides on each. What it
//! does not do is treat the ceiling as a single published contract with two
//! implementations.
//!
//! That distinction is the whole point of this module. Two entry points can each
//! be individually "covered" and still disagree — one rejecting 17 with
//! `BatchTooLarge` and the other trapping, or one accepting 16 while the other
//! off-by-ones at the boundary — and no test that exercises them separately
//! would notice, because neither test asserts anything about the *other* entry
//! point.
//!
//! So the assertions here are cross-cutting, and they come in three parts:
//!
//! 1. **The matrix.** Every batch entry point is driven with 0, 1, 16 and 17
//!    ids, and the outcome for a given size must be byte-identical across entry
//!    points — the same success, or the same typed [`Error`]. Not "an error".
//! 2. **The boundary is the constant.** 16 is accepted and 17 is rejected
//!    because `MAX_BATCH_SIZE` is 16, asserted against the constant itself, so
//!    changing the constant without changing the documented boundary fails
//!    here.
//! 3. **Discovery.** The set of entry points this module knows about is checked
//!    against the committed ABI inventory: every entry point that takes a batch
//!    argument must be one of them, and every one of them must be exercised.
//!    A third batch entry point added later therefore **fails this test by
//!    name** until it is wired into the matrix — the acceptance criterion that
//!    a future entry point "inherits the same check".
//!
//! # Rejection is structural, and uniform
//!
//! A batch that is empty or oversized is rejected before ids are looked up,
//! before authorization is checked and before anything is written. That is
//! asserted for both entry points here — no streams created, no events, no
//! balances moved — so "rejected identically" means identically in effect and
//! not merely in error code.

use std::format;
use std::string::{String, ToString};
use std::vec::Vec;

use soroban_sdk::testutils::Events as _;

use super::common::*;
use crate::{Error, MAX_BATCH_SIZE};

/// The committed ABI inventory: the source of truth for which entry points
/// exist and what they take.
const ABI_JSON: &str = include_str!("../../abi/fluxora_stream.json");

/// The batch sizes the ceiling is defined at: empty, single, exactly the cap,
/// and one past it.
const SIZES: [usize; 4] = [0, 1, 16, 17];

/// A batch entry point, identified by the name the ABI exports it under.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EntryPoint {
    BatchWithdraw,
    BatchExtendTtl,
}

impl EntryPoint {
    /// Every batch entry point that exists today. Kept in sync with the ABI by
    /// [`the_matrix_covers_every_entry_point_the_abi_advertises`].
    const ALL: [EntryPoint; 2] = [EntryPoint::BatchWithdraw, EntryPoint::BatchExtendTtl];

    fn name(self) -> &'static str {
        match self {
            EntryPoint::BatchWithdraw => "batch_withdraw",
            EntryPoint::BatchExtendTtl => "batch_extend_ttl",
        }
    }

    /// One line describing what a *successful* call at this size did, so the
    /// matrix can compare outcomes across entry points that return different
    /// types.
    fn call(self, h: &Harness, ids: &[u64]) -> Result<String, Error> {
        let ids = h.ids(ids);
        match self {
            EntryPoint::BatchWithdraw => match h.client.try_batch_withdraw(&h.recipient, &ids) {
                Ok(Ok(total)) => Ok(format!("paid {total}")),
                Err(Ok(err)) => Err(err),
                other => panic!("batch_withdraw: untyped outcome {other:?}"),
            },
            // `batch_extend_ttl` does not authenticate: it takes no party and
            // only touches TTL, so there is nothing to authorize.
            EntryPoint::BatchExtendTtl => match h.client.try_batch_extend_ttl(&ids) {
                Ok(Ok(extended)) => Ok(format!("extended {extended}")),
                Err(Ok(err)) => Err(err),
                other => panic!("batch_extend_ttl: untyped outcome {other:?}"),
            },
        }
    }

    /// The pre-flight a rejected batch must never get past: nothing created,
    /// nothing emitted, nothing moved.
    fn assert_no_side_effects(self, h: &Harness, label: &str) {
        assert_eq!(
            h.client.stream_count(),
            0,
            "{label}: a rejected batch must not involve any stream",
        );
        assert_eq!(
            stream_events(h),
            0,
            "{label}: a rejected batch must emit no stream events",
        );
        assert_eq!(h.pool(), 0, "{label}: a rejected batch must move no tokens");
    }
}

fn stream_events(h: &Harness) -> usize {
    h.env
        .events()
        .all()
        .filter_by_contract(&h.contract_id)
        .events()
        .len()
}

/// A harness with `n` live streams and enough time elapsed that every one has
/// something available — so a batch that is *accepted* can be observed doing
/// real work rather than being accepted vacuously.
fn harness_with(n: usize) -> Harness<'static> {
    let h = Harness::new();
    for _ in 0..n {
        h.create_simple(10 * ONE, 100 * DAY);
    }
    h.advance(50 * DAY);
    h.assert_pool_exact();
    h
}

fn ids_of(n: usize) -> Vec<u64> {
    (0..n as u64).collect()
}

// ---------------------------------------------------------------------------
// 1. The matrix
// ---------------------------------------------------------------------------

/// An empty batch is `EmptyBatch` on every entry point, and does nothing.
#[test]
fn every_entry_point_rejects_an_empty_batch_identically() {
    for entry in EntryPoint::ALL {
        let h = Harness::new();
        let result = entry.call(&h, &[]);
        assert_eq!(
            result,
            Err(Error::EmptyBatch),
            "{}: an empty batch must be EmptyBatch",
            entry.name(),
        );
        entry.assert_no_side_effects(&h, entry.name());
    }
}

/// A batch of exactly one is accepted on every entry point.
#[test]
fn every_entry_point_accepts_a_batch_of_one() {
    for entry in EntryPoint::ALL {
        let h = harness_with(1);
        let result = entry.call(&h, &[0]);
        assert!(
            result.is_ok(),
            "{}: a single-id batch must be accepted, got {result:?}",
            entry.name(),
        );
        h.assert_pool_exact();
    }
}

/// A batch of exactly [`MAX_BATCH_SIZE`] is accepted on every entry point —
/// the cap is inclusive, and it means the same thing everywhere.
#[test]
fn every_entry_point_accepts_a_batch_of_exactly_the_cap() {
    for entry in EntryPoint::ALL {
        let h = harness_with(MAX_BATCH_SIZE as usize);
        let result = entry.call(&h, &ids_of(MAX_BATCH_SIZE as usize));
        assert!(
            result.is_ok(),
            "{}: {MAX_BATCH_SIZE} ids is the published cap and must be accepted, \
             got {result:?}",
            entry.name(),
        );
        h.assert_pool_exact();
    }
}

/// One past the cap is `BatchTooLarge` on every entry point — not a trap, not a
/// different error, and with no side effects.
#[test]
fn every_entry_point_rejects_one_past_the_cap_with_the_same_error() {
    let n = MAX_BATCH_SIZE as usize + 1;
    for entry in EntryPoint::ALL {
        let h = Harness::new();
        // No streams exist: if the ceiling were checked after id lookup, this
        // would report StreamNotFound instead of BatchTooLarge.
        let result = entry.call(&h, &ids_of(n));
        assert_eq!(
            result,
            Err(Error::BatchTooLarge),
            "{}: {n} ids must be BatchTooLarge",
            entry.name(),
        );
        entry.assert_no_side_effects(&h, entry.name());
    }
}

/// The cross-entry-point claim, stated once: for every size in the matrix, all
/// entry points agree on the typed outcome.
///
/// This is the assertion that a divergence between the two implementations
/// cannot pass: it compares the packed outcomes against each other rather than
/// against a per-entry-point expectation.
#[test]
fn the_ceiling_is_enforced_identically_across_every_entry_point() {
    for n in SIZES {
        let mut agreed: Option<(String, Result<String, Error>)> = None;

        for entry in EntryPoint::ALL {
            let h = if n == 0 {
                Harness::new()
            } else {
                harness_with(n)
            };
            let ids = ids_of(n);
            let outcome = entry.call(&h, &ids);

            // Normalise run-specific values: a payout total is not comparable
            // across a differently-sized fixture, but the *shape* of the
            // outcome is.
            let shape = outcome.map(|_| "ok".to_string());
            match &agreed {
                None => agreed = Some((entry.name().to_string(), shape)),
                Some((first, expected)) => assert_eq!(
                    &shape,
                    expected,
                    "batch size {n}: `{}` gave {shape:?} but `{first}` gave \
                     {expected:?} — the ceiling must mean the same thing on \
                     every batch entry point",
                    entry.name(),
                ),
            }
        }

        // And for the sizes the ceiling exists to discriminate, pin the shape
        // rather than only its consistency.
        let (_, shape) = agreed.unwrap();
        match n {
            0 => assert_eq!(shape, Err(Error::EmptyBatch)),
            17 => assert_eq!(shape, Err(Error::BatchTooLarge)),
            _ => assert_eq!(shape, Ok("ok".to_string()), "size {n} must be accepted"),
        }
    }
}

// ---------------------------------------------------------------------------
// 2. The boundary is the constant
// ---------------------------------------------------------------------------

/// The published ceiling is the constant, from both sides, on both entry
/// points — so moving `MAX_BATCH_SIZE` without moving the documented boundary
/// fails rather than passing at the new value.
#[test]
fn the_boundary_is_the_declared_constant() {
    assert_eq!(
        MAX_BATCH_SIZE, 16,
        "docs and the batch entry points publish 16; if this changes, the \
         boundary tests below must change with it",
    );

    for entry in EntryPoint::ALL {
        let at_cap = harness_with(MAX_BATCH_SIZE as usize);
        assert!(
            entry
                .call(&at_cap, &ids_of(MAX_BATCH_SIZE as usize))
                .is_ok(),
            "{}: exactly MAX_BATCH_SIZE must be accepted",
            entry.name(),
        );
        at_cap.assert_pool_exact();

        let over_cap = Harness::new();
        assert_eq!(
            entry.call(&over_cap, &ids_of(MAX_BATCH_SIZE as usize + 1)),
            Err(Error::BatchTooLarge),
            "{}: MAX_BATCH_SIZE + 1 must be rejected",
            entry.name(),
        );
    }
}

/// The ceiling is applied before the ids are resolved, so an oversized batch of
/// entirely unknown ids is `BatchTooLarge` rather than `StreamNotFound`. Both
/// entry points must agree on that ordering, since it is what lets a caller
/// distinguish "my batch is too big" from "my batch is wrong".
#[test]
fn the_ceiling_is_checked_before_ids_are_resolved() {
    for entry in EntryPoint::ALL {
        let h = Harness::new();
        let unknown: Vec<u64> = (900..900 + MAX_BATCH_SIZE as u64 + 1).collect();
        assert_eq!(
            entry.call(&h, &unknown),
            Err(Error::BatchTooLarge),
            "{}: the size check must precede id lookup",
            entry.name(),
        );
    }
}

/// The empty-batch check is structural too, and identically so: a caller
/// presenting no authorization at all still gets `EmptyBatch` rather than an
/// auth failure, on both entry points. `test::batch` asserts this for
/// `batch_withdraw` alone; the point here is that the ordering is shared.
#[test]
fn the_emptiness_check_precedes_authorization_on_every_entry_point() {
    for entry in EntryPoint::ALL {
        let h = Harness::new();
        // Strip every auth entry: a call that reached `require_auth` could not
        // succeed, so a typed `EmptyBatch` proves the structural check ran
        // first.
        h.env.mock_auths(&[]);
        assert_eq!(
            entry.call(&h, &[]),
            Err(Error::EmptyBatch),
            "{}: the emptiness check must precede authorization",
            entry.name(),
        );
        assert!(
            h.env.auths().is_empty(),
            "{}: no authorization may be consumed by a rejected batch",
            entry.name(),
        );
    }
}

// ---------------------------------------------------------------------------
// 3. Discovery — a new batch entry point must join the matrix
// ---------------------------------------------------------------------------

/// Every entry point the ABI advertises as taking a batch argument is one this
/// module exercises — and this module exercises nothing that is not a batch
/// entry point.
///
/// A future `batch_*` entry point taking `Vec<u64>` fails here by name until it
/// is added to [`EntryPoint::ALL`], which is what makes the ceiling inherited
/// rather than re-implemented.
#[test]
fn the_matrix_covers_every_entry_point_the_abi_advertises() {
    let parsed: serde_json::Value =
        serde_json::from_str(ABI_JSON).expect("the committed ABI inventory is valid JSON");
    let functions = parsed["functions"]
        .as_array()
        .expect("the ABI inventory has a functions array");

    // A "batch argument" is any `Vec<...>` input: there is no batch-shaped
    // entry point that takes one id at a time.
    let mut batch_entry_points: Vec<String> = Vec::new();
    for f in functions {
        let name = f["name"].as_str().expect("every entry point has a name");
        let takes_a_vec = f["inputs"]
            .as_array()
            .expect("every entry point has an inputs array")
            .iter()
            .any(|i| i["type"].as_str().is_some_and(|t| t.starts_with("Vec<")));
        if takes_a_vec {
            batch_entry_points.push(name.to_string());
        }
    }

    assert!(
        !batch_entry_points.is_empty(),
        "the ABI advertises no batch entry point, but the contract exports two",
    );

    let covered: Vec<String> = EntryPoint::ALL
        .iter()
        .map(|e| e.name().to_string())
        .collect();
    for name in &batch_entry_points {
        assert!(
            covered.contains(name),
            "`{name}` takes a batch argument but is not exercised by the \
             MAX_BATCH_SIZE matrix in this module — add it to \
             `EntryPoint::ALL` so the ceiling is enforced identically",
        );
    }
    for name in &covered {
        assert!(
            batch_entry_points.contains(name),
            "this module exercises `{name}`, which no longer takes a batch \
             argument — the matrix is describing an entry point that is gone",
        );
    }
}
