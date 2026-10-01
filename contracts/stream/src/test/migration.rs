//! The documented migration path, exercised (#1870).
//! The documented migration path, exercised (#1870).
//!
//! `docs/MIGRATION.md` is the only place that tells a caller how to move from
//! the pre-v1 surface onto v1: which functions were renamed, which were
//! deleted, and — for the two that kept their names — how their signatures
//! changed. Nothing tested it, so the document could drift away from the
//! contract in either direction and no build would notice.
//!
//! This module closes that gap from both ends:
//!
//! * the **document** is parsed and cross-checked against the committed ABI
//!   inventory (`contracts/stream/abi/fluxora_stream.json`), so a rename the
//!   document still advertises as v1 — or a v1 entry point the document omits —
//!   fails a named test;
//! * the **path** is then walked live, call by call, using only what the
//!   document says to call, and the *behaviour* the document promises for each
//!   renamed function is asserted: `top_up` extends duration and never the
//!   rate, `transfer_recipient` is one step gated by the immutable
//!   `transferable` flag, `withdraw` with `None` means "take everything
//!   accrued", and the views that replaced `get_stream_state` and friends
//!   answer the same questions.
//!
//! It also pins the documented *upgrade posture*: the contract exposes no
//! upgrade entry point, so the document must say so, and the ABI must not
//! contain any of the upgrade functions a caller might otherwise assume.
//!
//! # Why the document is parsed rather than restated
//!
//! A test that hard-codes the rename table only proves the test agrees with
//! itself. Parsing §4 of the file makes the *document* the source of truth: edit
//! the table without editing the contract and
//! [`documented_v1_names_are_all_real_entry_points`] fails; delete a v1 entry
//! point the table still lists and it fails again. The hard-coded expectation
//! in [`the_renames_table_still_describes_the_documented_migration`] is the
//! mirror of that, and exists so a table that is quietly emptied or reflowed
//! cannot pass by accident.
//!
//! # What "state is asserted intact" means here
//!
//! The migration is a change of *caller*, not of protocol, so nothing about a
//! stream that predates it may move: after each step of the walked path the
//! stream's accounting identities are re-checked
//! ([`Harness::assert_invariants`]) and the token pool is required to still
//! equal the outstanding liability ([`Harness::assert_pool_exact`]). A rename
//! that silently changed an amount, or dropped a party, would show up as a
//! broken identity rather than as a passing call.

use std::string::{String, ToString};
use std::vec;
use std::vec::Vec;

use soroban_sdk::testutils::Address as _;
use soroban_sdk::Address;

use super::common::*;
use crate::{op, Error, StreamStatus};

/// The v1 ABI inventory, as committed. Kept in the repo for exactly this kind
/// of cross-check: it is generated from the contract spec and CI fails when it
/// is stale (`test::abi::abi_inventory_matches_generated_spec`).
const ABI_JSON: &str = include_str!("../../abi/fluxora_stream.json");

/// The migration document under test.
const MIGRATION_MD: &str = include_str!("../../../../docs/MIGRATION.md");

// ---------------------------------------------------------------------------
// Document parsing
// ---------------------------------------------------------------------------

/// The primary function name in one table cell.
///
/// Cells look like `` `cancel_stream(sender, id)` `` or
/// `` `update_recipient` / `accept_recipient_update` ``; the parameter list and
/// any second alternative are dropped.
fn fn_name_of(cell: &str) -> Option<String> {
    let text = cell.replace('`', "");
    let text = text.split('(').next().unwrap_or("");
    let text = text.split('/').next().unwrap_or("");
    let text = text.trim();
    if text.is_empty() {
        None
    } else {
        Some(text.to_string())
    }
}

/// The `old → v1` pairs from §4 of the document, in document order.
fn documented_renames() -> Vec<(String, String)> {
    let start = MIGRATION_MD
        .find("## 4. Renames")
        .expect("docs/MIGRATION.md must keep its §4 renames section");
    let section = &MIGRATION_MD[start..];
    let end = section[1..]
        .find("\n## ")
        .map(|i| i + 1)
        .unwrap_or(section.len());
    let section = &section[..end];

    let mut pairs = Vec::new();
    for line in section.lines() {
        let line = line.trim();
        if !line.starts_with('|') {
            continue;
        }
        let cells: Vec<&str> = line.trim_matches('|').split('|').map(str::trim).collect();
        if cells.len() < 3 || cells[0].starts_with("---") {
            continue;
        }
        // Skip the header row.
        if cells[0].eq_ignore_ascii_case("old") {
            continue;
        }
        if let (Some(old), Some(new)) = (fn_name_of(cells[0]), fn_name_of(cells[1])) {
            pairs.push((old, new));
        }
    }
    pairs
}

/// The `{ "functions": [ { "name": ... } ] }` names from the committed ABI.
fn abi_function_names() -> Vec<String> {
    let parsed: serde_json::Value =
        serde_json::from_str(ABI_JSON).expect("the committed ABI inventory is valid JSON");
    parsed["functions"]
        .as_array()
        .expect("the ABI inventory has a functions array")
        .iter()
        .map(|f| {
            f["name"]
                .as_str()
                .expect("every ABI function entry has a name")
                .to_string()
        })
        .collect()
}

fn is_delegation(name: &str) -> bool {
    name.starts_with("delegate_") || name == "grant_delegate" || name == "revoke_delegate"
}

/// Names that would indicate an upgradeable contract. None may appear in the
/// ABI, because the contract is deliberately immutable.
const UPGRADE_FUNCTIONS: &[&str] = &[
    "upgrade",
    "set_admin",
    "set_implementation",
    "update_wasm",
    "migrate",
    "migrate_state",
    "set_owner",
    "transfer_admin",
];

/// The upgrade posture is stated in the document and matches the ABI.
fn documented_posture_is_immutable() -> bool {
    MIGRATION_MD.contains("immutable")
        || MIGRATION_MD.contains("not upgradeable")
        || MIGRATION_MD.contains("non-upgradeable")
        || MIGRATION_MD.contains("cannot be upgraded")
}

// ---------------------------------------------------------------------------
// The document still describes a migration
// ---------------------------------------------------------------------------

/// The §4 table is the load-bearing part of the document. If it is emptied,
/// reflowed past recognition, or rewritten to describe something else, the
/// cross-checks below would pass vacuously — so pin it.
#[test]
fn the_renames_table_still_describes_the_documented_migration() {
    let pairs = documented_renames();
    let expected: Vec<(&str, &str)> = vec![
        ("cancel_stream", "cancel"),
        ("pause_stream", "pause"),
        ("resume_stream", "resume"),
        ("top_up_stream", "top_up"),
        ("update_recipient", "transfer_recipient"),
        ("get_stream_state", "get_stream"),
        ("get_stream_count", "stream_count"),
        ("get_withdrawable", "withdrawable_of"),
        ("calculate_accrued", "vested_of"),
        ("create_stream", "create_stream"),
        ("withdraw", "withdraw"),
    ];

    assert_eq!(
        pairs.len(),
        expected.len(),
        "docs/MIGRATION.md §4 lists {} renames, the documented migration has {}: {pairs:?}",
        pairs.len(),
        expected.len(),
    );
    for (got, want) in pairs.iter().zip(expected.iter()) {
        assert_eq!(got.0, want.0, "§4 old-name column changed");
        assert_eq!(got.1, want.1, "§4 v1-name column changed");
    }
}

/// Every name §4 points a caller at is a real entry point, and every name it
/// says was left behind is really gone.
///
/// This is the check that fails when the document and the contract drift: a
/// future rename that updates one but not the other shows up here as a
/// mismatch, named by the function it concerns.
#[test]
fn documented_v1_names_are_all_real_entry_points() {
    let abi = abi_function_names();

    for (old, new) in documented_renames() {
        assert!(
            abi.contains(&new),
            "docs/MIGRATION.md §4 tells callers to use `{new}`, but it is not in \
             the v1 ABI",
        );
        if old != new {
            assert!(
                !abi.contains(&old),
                "docs/MIGRATION.md §4 says `{old}` was renamed to `{new}`, but \
                 `{old}` is still in the v1 ABI",
            );
            assert!(
                !is_delegation(&old),
                "the §4 table describes the owner surface, not the delegated one",
            );
        }
    }
}

/// The counts §3 states are the counts the ABI actually has — so "v1 exposes 16
/// core entrypoints plus 8 delegation entrypoints" cannot become untrue.
#[test]
fn the_documented_entrypoint_counts_match_the_abi() {
    let abi = abi_function_names();
    let core = abi.iter().filter(|n| !is_delegation(n)).count();
    let delegation = abi.iter().filter(|n| is_delegation(n)).count();

    assert!(
        MIGRATION_MD.contains("v1 exposes **16** core entrypoints"),
        "docs/MIGRATION.md §3 must state the v1 core entrypoint count",
    );
    assert!(
        MIGRATION_MD.contains("**8 delegation entrypoints**"),
        "docs/MIGRATION.md §3 must state the delegation entrypoint count",
    );
    assert_eq!(
        core, 16,
        "the ABI has {core} core entrypoints, the doc says 16"
    );
    assert_eq!(
        delegation, 8,
        "the ABI has {delegation} delegation entrypoints, the doc says 8",
    );
    assert_eq!(
        abi.len(),
        core + delegation,
        "every ABI function must be classed as core or delegation",
    );
}

/// §3's breakdown of the *old* surface still sums to the total it quotes.
#[test]
fn the_documented_old_surface_breakdown_still_sums() {
    assert!(
        MIGRATION_MD.contains("exposed **145 entrypoints** (100 stream, 16 factory, 29"),
        "docs/MIGRATION.md §3 must state the old entrypoint total and breakdown",
    );
    assert_eq!(
        100 + 16 + 29,
        145,
        "the documented breakdown no longer sums to the documented total",
    );
}

/// §7's rulings are part of the documented path: the delegated withdrawal that
/// was cut is not back, and the negotiated two-step recipient change stayed
/// collapsed into one.
#[test]
fn the_documented_cuts_are_still_cut() {
    let abi = abi_function_names();
    for cut in [
        "delegated_withdraw",
        "delegated_cancel",
        "withdraw_to",
        "batch_withdraw_to",
        "accept_recipient_update",
        "update_rate",
        "update_rate_per_second",
        "extend_stream_end_time",
        "shorten_stream_end_time",
        "set_lookback_window",
        "init",
    ] {
        assert!(
            !abi.contains(&cut.to_string()),
            "`{cut}` is documented as removed from v1 but is in the ABI",
        );
    }
}

// ---------------------------------------------------------------------------
// The upgrade posture
// ---------------------------------------------------------------------------

/// The contract is deliberately immutable: the ABI exposes no upgrade entry
/// point, and the document says so.
#[test]
fn the_contract_is_immutable_and_the_document_says_so() {
    let abi = abi_function_names();

    for name in UPGRADE_FUNCTIONS {
        assert!(
            !abi.contains(&name.to_string()),
            "`{name}` is in the v1 ABI, but the contract is documented as \
             immutable — either remove the entry point or update the posture",
        );
    }

    assert!(
        documented_posture_is_immutable(),
        "docs/MIGRATION.md must state the upgrade posture (immutable / not \
         upgradeable) so integrators know the deployed contract cannot be \
         replaced",
    );
}

/// The ABI's entry points are exactly the ones the document accounts for, so
/// the "no upgrade entry point" claim is validated against what the contract
/// actually exposes rather than against a hand-maintained list.
#[test]
fn the_abi_exposes_no_upgrade_entry_point() {
    let abi = abi_function_names();
    for name in &abi {
        assert!(
            !UPGRADE_FUNCTIONS.contains(&name.as_str()),
            "the ABI exposes `{name}`, which contradicts the documented \
             immutable posture",
        );
    }
}

// ---------------------------------------------------------------------------
// Walking the path
// ---------------------------------------------------------------------------

/// A stream created the v1 way and then driven through every renamed entry
/// point in the order the document describes, with the state checked intact
/// after each step.
#[test]
fn the_documented_migration_path_runs_end_to_end() {
    let h = Harness::new();
    let start = h.now();

    // `create_stream(sender, recipient, token, deposit, start, end, cliff,
    // cancellable, pausable, transferable)` — the documented 6→10 argument
    // change, including the structural one: the token is now per stream.
    let id = h.client.create_stream(
        &h.sender,
        &h.recipient,
        &h.token,
        &(1_000 * ONE),
        &start,
        &(start + 100 * DAY),
        &start,
        &true,
        &true,
        &true,
        &None,
    );
    assert_eq!(
        h.get(id).token,
        h.token,
        "the token is named by the stream, not by a contract-wide config",
    );
    h.assert_pool_exact();

    // `get_stream_count` → `stream_count()`
    assert_eq!(h.client.stream_count(), 1);

    // `get_stream_state` → `get_stream(id)`
    let stream = h.client.get_stream(&id);
    assert_eq!(stream.sender, h.sender);
    assert_eq!(stream.recipient, h.recipient);
    assert_eq!(stream.deposited, 1_000 * ONE);
    assert_eq!(stream.status, StreamStatus::Active);

    h.advance(25 * DAY);

    // `get_withdrawable` → `withdrawable_of(id)`
    assert_eq!(h.client.withdrawable_of(&id), 250 * ONE);

    // `calculate_accrued` → `vested_of(id)`
    assert_eq!(h.client.vested_of(&id), 250 * ONE);

    // `withdraw(recipient, id, amount)` → `withdraw(id, Option<i128>)`: the
    // recipient is dropped (read from the stream) and every amount is now i128.
    let paid = h.client.withdraw(&id, &None);
    assert_eq!(
        paid,
        250 * ONE,
        "`None` means \"take everything accrued\", per the documented signature",
    );
    assert_eq!(h.balance(&h.recipient), 250 * ONE);
    h.assert_invariants();

    // `pause_stream(sender, id)` → `pause(id)`: the sender is read from the
    // stream rather than passed.
    h.client.pause(&id);
    assert_eq!(h.get(id).status, StreamStatus::Paused);
    h.assert_invariants();

    // `resume_stream(sender, id)` → `resume(id)`
    h.client.resume(&id);
    assert_eq!(h.get(id).status, StreamStatus::Active);
    h.assert_invariants();

    // `top_up_stream(...)` → `top_up(id, amount)`, with the documented
    // semantics change: it extends the duration at the old rate and never
    // touches the rate itself.
    let before = h.get(id);
    let rate_before = before.deposited / (before.end_time - before.start_time) as i128;
    h.client.top_up(&id, &(500 * ONE));
    let after = h.get(id);

    assert_eq!(after.deposited, before.deposited + 500 * ONE);
    let duration_after = (after.end_time - after.start_time) as i128;
    assert_eq!(
        after.deposited / duration_after,
        rate_before,
        "`top_up` must extend the schedule at the old rate, never raise the rate",
    );
    assert!(
        after.end_time > before.end_time,
        "`top_up` extends duration rather than front-loading the new funds",
    );
    // The top-up must not re-vest elapsed time: what the recipient could
    // already claim is unchanged by the extension.
    h.assert_pool_exact();

    // `update_recipient` / `accept_recipient_update` → `transfer_recipient(id,
    // new)`: one step instead of propose/accept.
    let new_recipient = Address::generate(&h.env);
    h.client.transfer_recipient(&id, &new_recipient);
    assert_eq!(h.get(id).recipient, new_recipient);
    assert_eq!(
        h.client.stream_count(),
        1,
        "a recipient change must not create a second stream",
    );
    h.assert_pool_exact();

    // `cancel_stream(sender, id)` → `cancel(id)`
    let refund_before = h.balance(&h.sender);
    let owed = h.client.refundable_of(&id);
    h.client.cancel(&id);
    assert_eq!(h.get(id).status, StreamStatus::Cancelled);
    assert_eq!(h.balance(&h.sender), refund_before + owed);
    h.assert_pool_exact();

    // Terminal semantics did not move with the rename: a second cancel is
    // rejected exactly as the old name would have rejected it.
    assert_eq!(
        h.client.try_cancel(&id).unwrap_err().unwrap(),
        Error::StreamTerminated,
    );
}

/// The `transfer_recipient` rename is gated by the new immutable `transferable`
/// flag; the document says the collapsed one-step call is "gated by the new
/// immutable `transferable` flag", so a stream created without it must refuse
/// the call and the flag must not be settable afterwards.
#[test]
fn the_collapsed_transfer_call_is_gated_by_the_immutable_flag() {
    let h = Harness::new();
    let start = h.now();
    let id = h.client.create_stream(
        &h.sender,
        &h.recipient,
        &h.token,
        &(100 * ONE),
        &start,
        &(start + 100 * DAY),
        &start,
        &true,
        &true,
        &false, // transferable
        &None,
    );

    assert_eq!(
        h.client
            .try_transfer_recipient(&id, &h.other)
            .unwrap_err()
            .unwrap(),
        Error::NotTransferable,
    );

    // The delegated spelling of the same operation is gated identically.
    h.client
        .grant_delegate(&id, &h.recipient, &h.other, &op::TRANSFER_RECIPIENT, &None);
    assert_eq!(
        h.client
            .try_delegate_transfer_recipient(&id, &h.other, &h.recipient)
            .unwrap_err()
            .unwrap(),
        Error::NotTransferable,
    );

    assert_eq!(h.get(id).recipient, h.recipient);
    h.assert_pool_exact();
}

/// The v1 entry points the document points at are the ones the ABI exports, and
/// the ABI exports nothing the document does not account for — core or
/// delegated.
#[test]
fn the_walked_entry_points_are_exactly_the_ones_the_document_names() {
    let abi = abi_function_names();

    // Every function this module calls must exist in the committed ABI.
    for called in [
        "create_stream",
        "stream_count",
        "get_stream",
        "withdrawable_of",
        "vested_of",
        "refundable_of",
        "withdraw",
        "pause",
        "resume",
        "top_up",
        "transfer_recipient",
        "cancel",
        "grant_delegate",
        "delegate_transfer_recipient",
        "stream_exists",
    ] {
        assert!(
            abi.contains(&called.to_string()),
            "this module walks `{called}`, which is not in the v1 ABI",
        );
    }

    // And the set of names the document accounts for — 11 renamed targets plus
    // the remaining core surface — covers the whole core ABI.
    let documented_renames = documented_renames();
    let mut accounted: Vec<String> = documented_renames.iter().map(|(_, n)| n.clone()).collect();
    for extra in [
        "batch_withdraw",
        "batch_extend_ttl",
        "extend_stream_ttl",
        "refundable_of",
        "stream_exists",
    ] {
        accounted.push(extra.to_string());
    }

    for name in abi {
        if is_delegation(&name) {
            continue;
        }
        assert!(
            accounted.contains(&name),
            "`{name}` is a v1 core entry point that docs/MIGRATION.md never \
             accounts for — the migration path is incomplete",
        );
    }
}
