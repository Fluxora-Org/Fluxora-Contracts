//! Upgrade posture: `FluxoraStream::upgradeable` (issue #2022).
//!
//! # What this suite is for
//!
//! `upgradeable` is documented as "the on-chain statement of the upgrade
//! posture documented in `docs/ABI.md` and `docs/MIGRATION.md`". Its doc
//! comment makes three claims, and until now nothing tested any of them:
//!
//! 1. the answer is `false` (the contract cannot be replaced in place);
//! 2. it is a constant, not a storage read, so it answers before any stream
//!    exists;
//! 3. it cannot be changed by the halt operator or anyone else.
//!
//! The published ABI snapshot carries the same statement as a top-level
//! `"upgradeable"` flag, so a fourth test pins that the snapshot and the
//! constant agree.
//!
//! # Failure mode guarded against
//!
//! Someone flips `UPGRADEABLE` to `true`, or drops the entry point in a
//! refactor. Integrators reading the published posture would then be misled
//! about whether the contract can be swapped underneath them. Every test below
//! fails if `UPGRADEABLE` is set to `true`.
//!
//! # Note on the ABI snapshot
//!
//! `upgradeable` is published in `contracts/stream/abi/fluxora_stream.json` as
//! the top-level `"upgradeable"` flag (next to `"upgrade_posture"`), not as an
//! entry in `"functions"`. That is deliberate: `script/check-abi-drift.py`
//! lists it in `EXCLUDE_FROM_ABI` as "internal flag, not a callable ABI
//! entrypoint". The snapshot test therefore asserts the flag, which is what
//! integrators actually read.

#![cfg(test)]

use super::common::*;
use crate::UPGRADEABLE;

/// The committed ABI inventory, the document integrators read.
const ABI_JSON: &str = include_str!("../../abi/fluxora_stream.json");

/// A fresh contract reports that it is not upgradeable, and answers before any
/// stream has been created (the doc comment's "constant, not a storage read"
/// claim). No halt operator is installed either, so nothing but the constant
/// can be producing the answer.
#[test]
fn fresh_contract_is_not_upgradeable_before_any_stream_exists() {
    let h = Harness::new();

    assert_eq!(
        h.client.stream_count(),
        0,
        "fixture precondition: no stream has been created"
    );
    assert_eq!(
        h.client.halt_operator(),
        None,
        "fixture precondition: no halt operator is installed"
    );

    assert!(
        !h.client.upgradeable(),
        "a fresh contract must report upgradeable() == false"
    );
    assert_eq!(
        h.client.upgradeable(),
        UPGRADEABLE,
        "the entry point must report the UPGRADEABLE constant"
    );
}

/// Installing the halt operator does not change the posture.
#[test]
fn set_halt_operator_does_not_change_upgradeable() {
    let h = Harness::new();
    assert!(!h.client.upgradeable(), "precondition: false before");

    h.client.set_halt_operator(&h.sender);

    assert_eq!(
        h.client.halt_operator(),
        Some(h.sender.clone()),
        "fixture precondition: the operator was installed"
    );
    assert!(
        !h.client.upgradeable(),
        "installing a halt operator must not make the contract upgradeable"
    );
}

/// Halting the contract does not change the posture, and the entry point
/// keeps answering while halted. Resuming leaves it unchanged as well.
#[test]
fn halt_and_resume_do_not_change_upgradeable() {
    let h = Harness::new();
    h.client.set_halt_operator(&h.sender);
    assert!(!h.client.upgradeable(), "precondition: false before halt");

    h.client.halt();
    assert!(
        h.client.halted(),
        "fixture precondition: the contract is halted"
    );
    assert!(
        !h.client.upgradeable(),
        "halting must not change upgradeable(), and the read must still answer"
    );

    h.client.resume_contract();
    assert!(
        !h.client.halted(),
        "fixture precondition: the contract is resumed"
    );
    assert!(
        !h.client.upgradeable(),
        "resuming must not change upgradeable()"
    );
}

/// The published ABI snapshot states the same posture as the constant: a
/// top-level `"upgradeable"` boolean, equal to `UPGRADEABLE` and `false`.
///
/// `functions` does not list `upgradeable` by design (see the module docs), so
/// the flag is the published entry this test pins.
#[test]
fn abi_snapshot_publishes_the_upgradeable_flag_as_false() {
    let snapshot: serde_json::Value =
        serde_json::from_str(ABI_JSON).expect("the committed ABI inventory is valid JSON");

    let flag = snapshot
        .get("upgradeable")
        .expect("the ABI snapshot must publish a top-level `upgradeable` entry")
        .as_bool()
        .expect("`upgradeable` in the ABI snapshot must be a boolean");

    assert!(!flag, "the ABI snapshot must publish upgradeable == false");
    assert_eq!(
        flag, UPGRADEABLE,
        "the ABI snapshot and the UPGRADEABLE constant must agree"
    );

    let posture = snapshot
        .get("upgrade_posture")
        .and_then(serde_json::Value::as_str)
        .expect("the ABI snapshot must carry a textual `upgrade_posture`");
    assert!(
        !posture.is_empty(),
        "`upgrade_posture` must state the posture in words"
    );
}
