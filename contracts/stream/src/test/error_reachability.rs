//! Issue #1689 — every error discriminant must be reachable, or reserved.
//!
//! # Why this module exists
//!
//! [`error_discriminants`] freezes each variant's number and drives a
//! `try_*` call to most of them, but nothing stopped a *new* variant from
//! landing in `error.rs` without ever being produced by an entry point or
//! being added to `docs/ABI.md`. The ABI table silently grew a row nobody
//! could reach — `VestedDecreased` (33) was merged in exactly that state.
//!
//! This module closes that hole with three checks:
//!
//! 1. **Compile-time classification** — [`describe`] is an exhaustive `match`
//!    over [`Error`] with **no wildcard arm**. Adding a variant to `error.rs`
//!    makes this file fail to compile, so `cargo test --workspace` (CI's test
//!    step) goes red until the new variant is classified here.
//! 2. **Reach** — every variant classified `Reach` is driven to by a public
//!    entry point in [`every_reachable_variant_is_produced_by_a_public_entry_point`],
//!    and the closure must return the *same* variant it claims to produce.
//! 3. **Reserve** — variants that cannot be produced are listed in the frozen
//!    [`RESERVED_DISCRIMINANTS`] allowlist with a reason, and those reasons are
//!    mirrored in `docs/ABI.md`. Expanding or shrinking the list is a
//!    deliberate, reviewable change.
//!
//! # Adding a new variant
//!
//! 1. Append it to `error.rs` with the next consecutive `= N`.
//! 2. Add a row to `DISCRIMINANT_FIXTURE`, bump `LAST_DISCRIMINANT`, and add
//!    the runtime row in `discriminant_fixture_matches_source`.
//! 3. Add a `describe` arm **and** an entry in [`ALL`]. Step 3 will not
//!    compile without step 3's array entry either — `names_and_discriminants_match_the_abi_fixture`
//!    asserts `ALL.len() == DISCRIMINANT_FIXTURE.len()`.
//! 4. If the variant is genuinely unproducible, add its discriminant to
//!    [`RESERVED_DISCRIMINANTS`] with a reason that also goes into
//!    `docs/ABI.md`. Otherwise write a `Reach` closure.
//! 5. Document the condition in `docs/ABI.md` "Error".
//!
//! # What "reachable" means here
//!
//! The closure runs against a fresh [`Harness`] and returns the variant a
//! *client* observes from a public `try_*` call. Injecting fixture state
//! (the stream-id counter, a rewritten stream entry) is allowed, because the
//! call under test still goes through the contract's public ABI; only the
//! precondition is arranged directly.

use soroban_sdk::testutils::Address as _;
use soroban_sdk::Address;

use super::common::*;
use super::create::seed_counter;
use super::error_discriminants::DISCRIMINANT_FIXTURE;
use super::token_errors::register_fee_on_transfer_token;
use crate::{op, Error};

// ---------------------------------------------------------------------------
// Classification
// ---------------------------------------------------------------------------

/// How the suite accounts for one variant.
enum Account {
    /// A public entry point drives the contract to this exact variant.
    ///
    /// The closure is built against a fresh [`Harness`] and returns whatever
    /// the client observed, so a regression that changes which variant the
    /// contract returns is caught even if the discriminant itself is stable.
    Reach(fn(&Harness) -> Error),

    /// Not producible from here. The reason is asserted non-empty and must be
    /// restated in `docs/ABI.md`, so a "reserved" classification cannot be
    /// used as a silent dump for an untested path.
    Reserved(&'static str),
}

/// The complete variant list, in discriminant order.
///
/// Kept as a value so the tests can iterate it. `names_and_discriminants_match_the_abi_fixture`
/// pins its length to `DISCRIMINANT_FIXTURE`, which is itself pinned to
/// `LAST_DISCRIMINANT`, so dropping an entry here fails the suite.
const ALL: [Error; 33] = [
    Error::StreamNotFound,
    Error::InvalidTimeRange,
    Error::InvalidCliff,
    Error::InvalidDeposit,
    Error::DepositRateTooLow,
    Error::SelfStream,
    Error::Unauthorized,
    Error::NotCancellable,
    Error::NotPausable,
    Error::NotTransferable,
    Error::StreamNotActive,
    Error::StreamNotPaused,
    Error::StreamAlreadyPaused,
    Error::StreamTerminated,
    Error::StreamMatured,
    Error::InsufficientWithdrawable,
    Error::NothingToWithdraw,
    Error::InvalidAmount,
    Error::BatchTooLarge,
    Error::EmptyBatch,
    Error::DuplicateStreamId,
    Error::Overflow,
    Error::TopUpTooSmall,
    Error::StreamIdExhausted,
    Error::TokenTransferFailed,
    Error::TokenMissing,
    Error::DelegateNotPermitted,
    Error::DelegateExpired,
    Error::MalformedStreamId,
    Error::RepeatedTransfer,
    Error::InvalidTopUp,
    Error::TokenAmountMismatch,
    Error::VestedDecreased,
];

/// Frozen allowlist of discriminants that have no reaching test, in ascending
/// order. Every entry must carry a reason in [`describe`] and a matching line
/// in `docs/ABI.md` "Error".
const RESERVED_DISCRIMINANTS: [u32; 5] = [11, 26, 29, 31, 33];

/// Name, discriminant, and account for a variant.
///
/// Exhaustive on purpose: no `_ =>` arm, so a new `error.rs` variant is a
/// compile error here until it is classified.
fn describe(e: Error) -> (&'static str, u32, Account) {
    match e {
        // --- Lookup -------------------------------------------------------
        Error::StreamNotFound => (
            "StreamNotFound",
            1,
            Account::Reach(|h| h.client.try_get_stream(&999).unwrap_err().unwrap()),
        ),

        // --- Creation validation ------------------------------------------
        Error::InvalidTimeRange => (
            "InvalidTimeRange",
            2,
            Account::Reach(|h| {
                let now = h.now();
                h.client
                    .try_create_stream(
                        &h.sender,
                        &h.recipient,
                        &h.token,
                        &(1_000 * ONE),
                        &now,
                        &now,
                        &now,
                        &true,
                        &true,
                        &true,
                    )
                    .unwrap_err()
                    .unwrap()
            }),
        ),
        Error::InvalidCliff => (
            "InvalidCliff",
            3,
            Account::Reach(|h| {
                let now = h.now();
                h.client
                    .try_create_stream(
                        &h.sender,
                        &h.recipient,
                        &h.token,
                        &(1_000 * ONE),
                        &now,
                        &(now + DAY),
                        &(now - 1),
                        &true,
                        &true,
                        &true,
                    )
                    .unwrap_err()
                    .unwrap()
            }),
        ),
        Error::InvalidDeposit => (
            "InvalidDeposit",
            4,
            Account::Reach(|h| {
                let now = h.now();
                h.client
                    .try_create_stream(
                        &h.sender,
                        &h.recipient,
                        &h.token,
                        &0,
                        &now,
                        &(now + DAY),
                        &now,
                        &true,
                        &true,
                        &true,
                    )
                    .unwrap_err()
                    .unwrap()
            }),
        ),
        Error::DepositRateTooLow => (
            "DepositRateTooLow",
            5,
            Account::Reach(|h| {
                let now = h.now();
                h.client
                    .try_create_stream(
                        &h.sender,
                        &h.recipient,
                        &h.token,
                        &1,
                        &now,
                        &(now + DAY),
                        &now,
                        &true,
                        &true,
                        &true,
                    )
                    .unwrap_err()
                    .unwrap()
            }),
        ),
        Error::SelfStream => (
            "SelfStream",
            6,
            Account::Reach(|h| {
                let now = h.now();
                h.client
                    .try_create_stream(
                        &h.sender,
                        &h.sender,
                        &h.token,
                        &(1_000 * ONE),
                        &now,
                        &(now + DAY),
                        &now,
                        &true,
                        &true,
                        &true,
                    )
                    .unwrap_err()
                    .unwrap()
            }),
        ),

        // --- Authorization / capability ------------------------------------
        Error::Unauthorized => (
            "Unauthorized",
            7,
            Account::Reach(|h| {
                let id = h.create_simple(1_000 * ONE, 100 * DAY);
                h.advance(10 * DAY);
                h.client
                    .try_batch_withdraw(&h.other, &h.ids(&[id]))
                    .unwrap_err()
                    .unwrap()
            }),
        ),
        Error::NotCancellable => (
            "NotCancellable",
            8,
            Account::Reach(|h| {
                let now = h.now();
                let id = h.create(1_000 * ONE, now, now + DAY, now, false, true, true);
                h.client.try_cancel(&id).unwrap_err().unwrap()
            }),
        ),
        Error::NotPausable => (
            "NotPausable",
            9,
            Account::Reach(|h| {
                let now = h.now();
                let id = h.create(1_000 * ONE, now, now + DAY, now, true, false, true);
                h.client.try_pause(&id).unwrap_err().unwrap()
            }),
        ),
        Error::NotTransferable => (
            "NotTransferable",
            10,
            Account::Reach(|h| {
                let now = h.now();
                let id = h.create(1_000 * ONE, now, now + DAY, now, true, true, false);
                let other = Address::generate(&h.env);
                h.client
                    .try_transfer_recipient(&id, &other)
                    .unwrap_err()
                    .unwrap()
            }),
        ),

        // --- State machine -------------------------------------------------
        Error::StreamNotActive => (
            "StreamNotActive",
            11,
            Account::Reserved(
                "Superseded. The state-machine entry points each name their own \
                 condition — StreamNotPaused (12) on resume, StreamAlreadyPaused \
                 (13) on pause, StreamTerminated (14) on a finished stream — so no \
                 code path emits the generic 11. Kept only because it is frozen \
                 ABI: a client that already decodes 11 must keep seeing 11.",
            ),
        ),
        Error::StreamNotPaused => (
            "StreamNotPaused",
            12,
            Account::Reach(|h| {
                let id = h.create_simple(1_000 * ONE, 100 * DAY);
                h.client.try_resume(&id).unwrap_err().unwrap()
            }),
        ),
        Error::StreamAlreadyPaused => (
            "StreamAlreadyPaused",
            13,
            Account::Reach(|h| {
                let id = h.create_simple(1_000 * ONE, 100 * DAY);
                h.client.pause(&id);
                h.client.try_pause(&id).unwrap_err().unwrap()
            }),
        ),
        Error::StreamTerminated => (
            "StreamTerminated",
            14,
            Account::Reach(|h| {
                let id = h.create_simple(1_000 * ONE, 100 * DAY);
                h.client.cancel(&id);
                h.client.try_cancel(&id).unwrap_err().unwrap()
            }),
        ),
        Error::StreamMatured => (
            "StreamMatured",
            15,
            Account::Reach(|h| {
                let id = h.create_simple(1_000 * ONE, DAY);
                h.advance(DAY + 1);
                h.client.try_top_up(&id, &(100 * ONE)).unwrap_err().unwrap()
            }),
        ),

        // --- Withdrawal -----------------------------------------------------
        Error::InsufficientWithdrawable => (
            "InsufficientWithdrawable",
            16,
            Account::Reach(|h| {
                let id = h.create_simple(1_000 * ONE, 100 * DAY);
                h.advance(10 * DAY);
                h.client
                    .try_withdraw(&id, &Some(200 * ONE))
                    .unwrap_err()
                    .unwrap()
            }),
        ),
        Error::NothingToWithdraw => (
            "NothingToWithdraw",
            17,
            Account::Reach(|h| {
                let now = h.now();
                let id = h.create(
                    1_000 * ONE,
                    now + DAY,
                    now + 2 * DAY,
                    now + DAY,
                    true,
                    true,
                    true,
                );
                h.client.try_withdraw(&id, &None).unwrap_err().unwrap()
            }),
        ),
        Error::InvalidAmount => (
            "InvalidAmount",
            18,
            Account::Reach(|h| {
                let id = h.create_simple(1_000 * ONE, 100 * DAY);
                h.advance(10 * DAY);
                h.client.try_withdraw(&id, &Some(0)).unwrap_err().unwrap()
            }),
        ),

        // --- Resource limits -------------------------------------------------
        Error::BatchTooLarge => (
            "BatchTooLarge",
            19,
            Account::Reach(|h| {
                let ids: std::vec::Vec<u64> = (0..17).collect();
                let id_vec = soroban_sdk::Vec::from_slice(&h.env, &ids);
                h.client
                    .try_batch_withdraw(&h.recipient, &id_vec)
                    .unwrap_err()
                    .unwrap()
            }),
        ),
        Error::EmptyBatch => (
            "EmptyBatch",
            20,
            Account::Reach(|h| {
                h.client
                    .try_batch_withdraw(&h.recipient, &h.ids(&[]))
                    .unwrap_err()
                    .unwrap()
            }),
        ),
        Error::DuplicateStreamId => (
            "DuplicateStreamId",
            21,
            Account::Reach(|h| {
                let id = h.create_simple(1_000 * ONE, 100 * DAY);
                h.client
                    .try_batch_withdraw(&h.recipient, &h.ids(&[id, id]))
                    .unwrap_err()
                    .unwrap()
            }),
        ),

        // --- Arithmetic -------------------------------------------------------
        Error::Overflow => (
            "Overflow",
            22,
            Account::Reach(|h| {
                let now = h.now();
                let huge_deposit: i128 = i128::MAX / 2;
                h.token_admin.mint(&h.sender, &huge_deposit);
                h.client
                    .try_create_stream(
                        &h.sender,
                        &h.recipient,
                        &h.token,
                        &huge_deposit,
                        &now,
                        &(now + 3),
                        &now,
                        &true,
                        &true,
                        &true,
                    )
                    .unwrap_err()
                    .unwrap()
            }),
        ),
        Error::TopUpTooSmall => (
            "TopUpTooSmall",
            23,
            Account::Reach(|h| {
                let id = h.create_simple(1_000 * ONE, DAY);
                h.client.try_top_up(&id, &1).unwrap_err().unwrap()
            }),
        ),

        // --- Identifier exhaustion ---------------------------------------------
        Error::StreamIdExhausted => (
            "StreamIdExhausted",
            24,
            Account::Reach(|h| {
                // Arrange the precondition directly: creating u64::MAX streams
                // in a unit test is not viable, but the entry point under test
                // is still `create_stream`.
                seed_counter(h, u64::MAX);
                let now = h.now();
                h.client
                    .try_create_stream(
                        &h.sender,
                        &h.recipient,
                        &h.token,
                        &(1_000 * ONE),
                        &now,
                        &(now + 100 * DAY),
                        &now,
                        &true,
                        &true,
                        &true,
                    )
                    .unwrap_err()
                    .unwrap()
            }),
        ),

        // --- Token sub-invocation ------------------------------------------------
        Error::TokenTransferFailed => (
            "TokenTransferFailed",
            25,
            Account::Reach(|h| {
                // Drain the sender so the SAC rejects the deposit pull with a
                // contract-typed error.
                let balance = h.balance(&h.sender);
                h.token_client.transfer(&h.sender, &h.other, &balance);
                let now = h.now();
                h.client
                    .try_create_stream(
                        &h.sender,
                        &h.recipient,
                        &h.token,
                        &(1_000 * ONE),
                        &now,
                        &(now + 100 * DAY),
                        &now,
                        &true,
                        &true,
                        &true,
                    )
                    .unwrap_err()
                    .unwrap()
            }),
        ),
        Error::TokenMissing => (
            "TokenMissing",
            26,
            Account::Reserved(
                "Real-network only. 26 maps `InvokeError::Abort` — a host trap \
                 that is not a contract error, e.g. calling `transfer` on a token \
                 address with no deployed code. Under the native test host every \
                 sub-invocation failure is contract-typed, so 26 collapses into \
                 25 and cannot be produced here. See the module docs of \
                 test::token_errors.",
            ),
        ),

        // --- Delegation -------------------------------------------------------------
        Error::DelegateNotPermitted => (
            "DelegateNotPermitted",
            27,
            Account::Reach(|h| {
                let id = h.create_simple(1_000 * ONE, 100 * DAY);
                let agent = Address::generate(&h.env);
                h.advance(10 * DAY);
                h.client
                    .try_delegate_withdraw(&id, &agent, &None)
                    .unwrap_err()
                    .unwrap()
            }),
        ),
        Error::DelegateExpired => (
            "DelegateExpired",
            28,
            Account::Reach(|h| {
                let id = h.create_simple(1_000 * ONE, 100 * DAY);
                let agent = Address::generate(&h.env);
                let expires = h.now() + DAY;
                h.client
                    .grant_delegate(&id, &h.recipient, &agent, &op::WITHDRAW, &Some(expires));
                h.advance(DAY + 1);
                h.client
                    .try_delegate_withdraw(&id, &agent, &None)
                    .unwrap_err()
                    .unwrap()
            }),
        ),

        // --- Batch validation -----------------------------------------------------------
        Error::MalformedStreamId => (
            "MalformedStreamId",
            29,
            Account::Reserved(
                "Defence in depth for raw XDR callers. The typed client \
                 argument is `Vec<u64>`, so the host rejects a non-u64 element \
                 before the contract body runs; no well-typed public call can \
                 reach 29. Kept because an untyped on-chain caller can still \
                 send the malformed vector.",
            ),
        ),

        // --- Transfer -------------------------------------------------------------------
        Error::RepeatedTransfer => (
            "RepeatedTransfer",
            30,
            Account::Reach(|h| {
                let now = h.now();
                let id = h.create(1_000 * ONE, now, now + DAY, now, true, true, true);
                h.client
                    .try_transfer_recipient(&id, &h.recipient)
                    .unwrap_err()
                    .unwrap()
            }),
        ),

        // --- Arithmetic (top-up) -----------------------------------------------------------
        Error::InvalidTopUp => (
            "InvalidTopUp",
            31,
            Account::Reserved(
                "Superseded by InvalidAmount (18). `top_up` rejects \
                 `amount <= 0` with 18 before any schedule arithmetic runs, so \
                 no code path emits 31. Frozen ABI; see test::error_discriminants.",
            ),
        ),

        // --- Token assumptions ---------------------------------------------------------------
        Error::TokenAmountMismatch => (
            "TokenAmountMismatch",
            32,
            Account::Reach(|h| {
                let (token, fee_token) = register_fee_on_transfer_token(h);
                fee_token.set_fee_bps(&1_000);
                let now = h.now();
                h.client
                    .try_create_stream(
                        &h.sender,
                        &h.recipient,
                        &token,
                        &(1_000 * ONE),
                        &now,
                        &(now + 100 * DAY),
                        &now,
                        &true,
                        &true,
                        &true,
                    )
                    .unwrap_err()
                    .unwrap()
            }),
        ),

        // --- Vesting monotonicity ---------------------------------------------------------------
        Error::VestedDecreased => (
            "VestedDecreased",
            33,
            Account::Reserved(
                "Defensive monotonicity guard on pause, resume, top_up and \
                 transfer_recipient. `vested` is non-decreasing under every \
                 mutation those four paths can make — pause/resume keep elapsed \
                 time identical, `top_up` scales numerator and denominator \
                 together, and a recipient change does not enter the formula — \
                 so 33 is unreachable. The guard stays because the invariant it \
                 protects is load-bearing.",
            ),
        ),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// The classification is complete: names and numbers agree with the frozen ABI
/// fixture, and every variant's runtime discriminant is unchanged.
///
/// The *completeness* half of this test is the exhaustive `match` in
/// [`describe`] — a new variant is a compile error, not a silent omission.
#[test]
fn names_and_discriminants_match_the_abi_fixture() {
    assert_eq!(
        ALL.len(),
        DISCRIMINANT_FIXTURE.len(),
        "ALL and DISCRIMINANT_FIXTURE disagree on the number of variants — \
         a variant was added to one table and not the other",
    );

    for (i, e) in ALL.iter().enumerate() {
        let (name, disc, _) = describe(*e);
        let (fixture_name, fixture_disc) = DISCRIMINANT_FIXTURE[i];

        assert_eq!(
            name, fixture_name,
            "ALL[{i}] is {name} but DISCRIMINANT_FIXTURE[{i}] is \
             {fixture_name} — keep both tables in discriminant order",
        );
        assert_eq!(
            disc, fixture_disc,
            "DISCRIMINANT_FIXTURE[{i}] ({name}) records {fixture_disc} but \
             describe() reports {disc}",
        );
        assert_eq!(
            disc, *e as u32,
            "{name}: describe() says {disc}, `Error::{name} as u32` is {} — \
             error.rs was renumbered",
            *e as u32,
        );
    }
}

/// The set of variants with no reaching test is exactly [`RESERVED_DISCRIMINANTS`],
/// and each carries a non-empty reason.
///
/// Shrinking the list without adding a `Reach` closure, or growing it to hide
/// an unproducible path, is a deliberate diff against a frozen constant.
#[test]
fn reserved_allowlist_is_frozen_and_every_reservation_states_a_reason() {
    let mut reserved: std::vec::Vec<u32> = std::vec::Vec::new();
    let mut reachable = 0usize;

    for e in ALL {
        let (name, disc, account) = describe(e);
        match account {
            Account::Reach(_) => reachable += 1,
            Account::Reserved(reason) => {
                assert!(
                    !reason.trim().is_empty(),
                    "Error::{name} (#{disc}) is reserved without a reason",
                );
                reserved.push(disc);
            }
        }
    }

    reserved.sort_unstable();
    assert_eq!(
        reserved, RESERVED_DISCRIMINANTS,
        "the reserved set changed. Every reserved variant must be listed in \
         docs/ABI.md \"Error\" with the same reason; every reachable variant \
         must have a Reach closure in describe().",
    );

    // A handful of variants are reserved; the rest must be driven end-to-end.
    assert!(
        reachable >= ALL.len() - RESERVED_DISCRIMINANTS.len(),
        "reachable count {reachable} is lower than expected",
    );
}

/// Every variant classified `Reach` is actually produced by a public entry
/// point, and the observed variant is the claimed one.
///
/// This is the enforcement half of issue #1689's first acceptance criterion:
/// a variant that stops being produced — because a guard was reordered or a
/// check was inlined away — fails here even though its discriminant test in
/// `error_discriminants` (a bare `as u32` cast) would still pass.
#[test]
fn every_reachable_variant_is_produced_by_a_public_entry_point() {
    for e in ALL {
        let (name, disc, account) = describe(e);
        if let Account::Reach(produce) = account {
            let h = Harness::new();
            let observed = produce(&h);
            assert_eq!(
                observed, e,
                "Error::{name} (#{disc}) claimed reachable, but the public \
                 entry point returned {observed:?}",
            );
        }
    }
}

/// `docs/ABI.md` states a condition and a reachability status for every
/// variant, in the same order as the ABI fixture.
///
/// This is issue #1689's second acceptance criterion turned into a gate: a
/// variant whose row is missing, whose status disagrees with [`describe`],
/// or whose discriminant was renumbered fails here. The doc and the contract
/// cannot drift apart without a test noticing.
#[test]
fn abi_doc_states_a_condition_and_status_for_every_variant() {
    let path = std::format!("{}/../../docs/ABI.md", env!("CARGO_MANIFEST_DIR"));
    let doc = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {path}: {e}"));

    let section_start = doc
        .find("### `Error`")
        .unwrap_or_else(|| panic!("docs/ABI.md has no `### `Error`` section"));
    let section = &doc[section_start..];
    let section = section
        .split("\n---\n")
        .next()
        .expect("the Error section always terminates");

    // `| 1 | \`StreamNotFound\` | condition | reachable |`
    let mut rows: std::vec::Vec<(u32, &str, &str)> = std::vec::Vec::new();
    for line in section.lines() {
        let cells: std::vec::Vec<&str> = line.split('|').collect();
        if cells.len() < 5 {
            continue;
        }
        let Ok(disc) = cells[1].trim().parse::<u32>() else {
            continue;
        };
        let name = cells[2].trim().trim_matches('`');
        let status = cells[4].trim();
        rows.push((disc, name, status));
    }

    assert_eq!(
        rows.len(),
        ALL.len(),
        "docs/ABI.md `### Error` documents {} variants, expected {} — a \
         variant was added without a documented condition",
        rows.len(),
        ALL.len(),
    );

    for (i, e) in ALL.iter().enumerate() {
        let (name, disc, account) = describe(*e);
        let (doc_disc, doc_name, doc_status) = rows[i];
        let expected_status = match account {
            Account::Reach(_) => "reachable",
            Account::Reserved(_) => "reserved",
        };

        assert_eq!(
            doc_disc, disc,
            "docs/ABI.md row {i} is #{doc_disc}, expected #{disc} ({name})",
        );
        assert_eq!(
            doc_name, name,
            "docs/ABI.md row {i} is {doc_name}, expected {name}",
        );
        assert_eq!(
            doc_status, expected_status,
            "docs/ABI.md marks {name} (#{disc}) as {doc_status} but \
             test::error_reachability classifies it as {expected_status}",
        );
    }
}
