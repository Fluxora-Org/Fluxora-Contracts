//! Issue #1840 — a stream whose deposit is the maximum representable amount.
//!
//! `accrual::vested` computes `deposited * consumed` before dividing, so the
//! largest deposit is where that intermediate product presses hardest on the
//! `i128` ceiling. `create_stream` front-loads the identical guard against
//! `deposit * duration`, so the only schedule on which `deposit == i128::MAX`
//! survives validation is `duration == 1`: the guarded product is exactly
//! `i128::MAX`, and `vested` never reaches its multiplication because
//! `consumed >= duration` returns the deposit in full. That is the boundary
//! `docs/ABI.md` calls out when it documents `Overflow` (22) as
//! "defensive only … unreachable for any stream created through the contract".
//!
//! No module in the suite funds a stream with the maximum representable amount.
//! The shared harness mints only `1_000_000 * ONE`, so this module registers a
//! dedicated Stellar Asset Contract whose entire supply is `i128::MAX` and
//! drives the scenario end to end through the public ABI: create, view,
//! withdraw, deplete — asserting accounting, emitted events, token conservation
//! and the final stream state at every step. The largest deposit that is
//! *rejected* (its `deposit * duration` product past `i128::MAX`) is pinned
//! alongside it.

use soroban_sdk::testutils::Address as _;
use soroban_sdk::testutils::Events as _;
use soroban_sdk::token::{Client as TokenClient, StellarAssetClient};
use soroban_sdk::Address;
use soroban_sdk::Event as _;

use super::common::*;
use crate::events::{StreamCreated, Withdrawn};
use crate::{CliffMode, Error, ReleaseCurve, StreamStatus};

// ---------------------------------------------------------------------------
// Fixture: the harness plus a token whose whole supply is `i128::MAX`
// ---------------------------------------------------------------------------

/// [`Harness`] plus a dedicated asset minted with the full `i128` range.
///
/// A dedicated asset is required, not a convenience: [`Harness::new`] already
/// mints `1_000_000 * ONE` to both `sender` and `other`, and adding `i128::MAX`
/// on top of an existing supply would overflow the token's own ledger. A fresh
/// asset holds `i128::MAX` and nothing else, which is what makes the maximum
/// representable deposit reachable through the public ABI at all.
struct MaxDepositFixture<'a> {
    h: Harness<'a>,
    token: Address,
}

impl<'a> MaxDepositFixture<'a> {
    fn new() -> Self {
        let h = Harness::new();
        let issuer = Address::generate(&h.env);
        let token = h.env.register_stellar_asset_contract_v2(issuer).address();
        StellarAssetClient::new(&h.env, &token).mint(&h.sender, &i128::MAX);
        Self { h, token }
    }

    fn balance(&self, who: &Address) -> i128 {
        TokenClient::new(&self.h.env, &self.token).balance(who)
    }

    /// The events the *stream* contract published during the most recent
    /// invocation. Read immediately after the call under test — any later
    /// client call replaces the snapshot.
    fn stream_events(&self) -> std::vec::Vec<soroban_sdk::xdr::ContractEvent> {
        self.h
            .env
            .events()
            .all()
            .filter_by_contract(&self.h.contract_id)
            .events()
            .to_vec()
    }
}

// ---------------------------------------------------------------------------
// The scenario: a stream whose deposit is `i128::MAX`
// ---------------------------------------------------------------------------

/// The whole life of a stream funded with the maximum representable amount,
/// through the public ABI, with accounting, events, token balances and final
/// state asserted at each stage.
///
/// A one-second schedule is the only one that can hold `i128::MAX`: the
/// creation guard evaluates `i128::MAX * 1` exactly at the ceiling. Nothing
/// vests for that second (`consumed == 0`), and at the end instant the
/// `consumed >= duration` branch delivers the deposit in full — the guarded
/// multiplication is never reached, which is precisely why the max-deposit
/// stream cannot overflow.
#[test]
fn max_deposit_streams_and_settles_end_to_end_through_the_public_abi() {
    let f = MaxDepositFixture::new();
    let h = &f.h;

    let start = h.now();
    let end = start + 1;
    let deposit = i128::MAX;

    // --- create: the entire i128 range is escrowed -------------------------
    let id = h.client.create_stream(
        &h.sender,
        &h.recipient,
        &f.token,
        &deposit,
        &start,
        &end,
        &start,
        &true,
        &true,
        &true,
        &None,
    );
    assert_eq!(id, 0, "first stream id");

    let created = f.stream_events();
    let expected = StreamCreated {
        stream_id: id,
        sender: h.sender.clone(),
        recipient: h.recipient.clone(),
        token: f.token.clone(),
        deposited: deposit,
        start_time: start,
        end_time: end,
        cliff_time: start,
        cancellable: true,
        pausable: true,
        transferable: true,
        curve: ReleaseCurve::Linear,
        cliff_mode: CliffMode::Schedule,
        reference: None,
    };
    assert_eq!(
        created,
        std::vec![expected.to_xdr(&h.env, &h.contract_id)],
        "stream_created must carry the full i128::MAX deposit and the schedule",
    );

    let s = h.get(id);
    assert_eq!(s.deposited, deposit, "maximum representable deposit stored");
    assert_eq!(s.withdrawn, 0);
    assert_eq!(s.start_time, start);
    assert_eq!(s.end_time, end);
    assert_eq!(s.status, StreamStatus::Active);
    assert_eq!(f.balance(&h.contract_id), deposit, "pool holds the deposit");
    assert_eq!(f.balance(&h.sender), 0, "sender spent its entire supply");
    assert_eq!(f.balance(&h.recipient), 0);
    f.h.assert_pool_exact_for(&f.token);

    // --- the first instant: nothing elapsed, nothing withdrawable ----------
    assert_eq!(h.client.vested_of(&id), 0, "consumed is zero at start");
    assert_eq!(h.client.withdrawable_of(&id), 0);
    assert_eq!(h.client.refundable_of(&id), deposit);
    assert_eq!(
        h.client.vested_of(&id) + h.client.refundable_of(&id),
        deposit,
        "conservation at start",
    );

    let err = h.client.try_withdraw(&id, &None).unwrap_err().unwrap();
    assert_eq!(
        err,
        Error::NothingToWithdraw,
        "live stream with zero accrued is not payable",
    );

    // --- maturity: the full deposit vests in one step ----------------------
    h.warp_to(end);
    assert_eq!(h.client.vested_of(&id), deposit);
    assert_eq!(h.client.withdrawable_of(&id), deposit);
    assert_eq!(h.client.refundable_of(&id), 0);
    assert_eq!(
        h.client.vested_of(&id) + h.client.refundable_of(&id),
        deposit,
        "conservation at maturity",
    );

    let paid = h.client.withdraw(&id, &None);
    assert_eq!(paid, deposit, "withdraw returns the whole deposit");

    let drained = f.stream_events();
    let expected = Withdrawn {
        stream_id: id,
        recipient: h.recipient.clone(),
        amount: deposit,
        withdrawn: deposit,
        deposited: deposit,
        status: StreamStatus::Depleted,
        sender: h.sender.clone(),
        paused_at: None,
        paused_total: 0,
    };
    assert_eq!(
        drained,
        std::vec![expected.to_xdr(&h.env, &h.contract_id)],
        "withdrawn event must report the full deposit and the Depleted status",
    );

    // --- final state and token conservation --------------------------------
    let s = h.get(id);
    assert_eq!(s.withdrawn, deposit);
    assert_eq!(s.deposited, deposit);
    assert_eq!(s.status, StreamStatus::Depleted);

    assert_eq!(f.balance(&h.recipient), deposit, "recipient paid in full");
    assert_eq!(f.balance(&h.contract_id), 0, "pool drained to zero");
    assert_eq!(f.balance(&h.sender), 0);
    assert_eq!(
        f.balance(&h.sender) + f.balance(&h.recipient) + f.balance(&h.contract_id),
        deposit,
        "every stroop of the maximum supply is accounted for",
    );
    f.h.assert_pool_exact_for(&f.token);

    let err = h.client.try_withdraw(&id, &None).unwrap_err().unwrap();
    assert_eq!(
        err,
        Error::StreamTerminated,
        "a depleted max-deposit stream pays nothing further",
    );
    f.h.assert_pool_exact_for(&f.token);
}

/// `i128::MAX` is a valid deposit for exactly one schedule length.
///
/// Every duration above one second makes the creation guard's
/// `deposit * duration` product overflow, so it is rejected with the typed
/// [`Error::Overflow`] before any token moves or any id is consumed — never a
/// host trap, and never a partially funded stream.
#[test]
fn max_deposit_only_survives_a_one_second_schedule() {
    let f = MaxDepositFixture::new();
    let h = &f.h;

    let start = h.now();
    let deposit = i128::MAX;

    for duration in [2u64, 3, 4, 10, 1_000, 1 << 20] {
        let err = h
            .client
            .try_create_stream(
                &h.sender,
                &h.recipient,
                &f.token,
                &deposit,
                &start,
                &(start + duration),
                &start,
                &true,
                &true,
                &true,
                &None,
            )
            .unwrap_err()
            .unwrap();
        assert_eq!(
            err,
            Error::Overflow,
            "duration {duration}: i128::MAX * duration must be a typed overflow",
        );
    }

    assert_eq!(
        h.client.stream_count(),
        0,
        "a rejected create consumes no stream id",
    );
    assert_eq!(f.balance(&h.contract_id), 0, "no funds moved on rejection");
    assert_eq!(f.balance(&h.sender), deposit);

    // One second is the unique fit: `i128::MAX * 1` is exactly representable.
    let id = h.client.create_stream(
        &h.sender,
        &h.recipient,
        &f.token,
        &deposit,
        &start,
        &(start + 1),
        &start,
        &true,
        &true,
        &true,
        &None,
    );
    assert_eq!(id, 0);
    assert_eq!(f.balance(&h.contract_id), deposit);
    f.h.assert_pool_exact_for(&f.token);
}

/// The largest deposit a longer schedule can carry, and the first it cannot.
///
/// For a `duration`-second schedule the creation guard accepts
/// `floor(i128::MAX / duration)` and rejects one stroop more with
/// [`Error::Overflow`]. The accepted stream then drives accrual to its widest
/// representable numerator — `deposited * (duration - 1)`, one second before
/// maturity — which must divide with correct floor rounding rather than
/// overflow, exactly as `docs/ABI.md` documents for `vested_of`.
#[test]
fn largest_representable_deposit_is_the_creation_guard_boundary() {
    let f = MaxDepositFixture::new();
    let h = &f.h;

    let duration = 1u64 << 20;
    let d = duration as i128;
    let boundary = i128::MAX / d;
    let start = h.now();

    // One stroop past the boundary overflows the guard: typed, atomic, no id.
    let err = h
        .client
        .try_create_stream(
            &h.sender,
            &h.recipient,
            &f.token,
            &(boundary + 1),
            &start,
            &(start + duration),
            &start,
            &true,
            &true,
            &true,
            &None,
        )
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::Overflow);
    assert_eq!(h.client.stream_count(), 0);
    assert_eq!(f.balance(&h.contract_id), 0);

    // The boundary itself is accepted and fully funded.
    let id = h.client.create_stream(
        &h.sender,
        &h.recipient,
        &f.token,
        &boundary,
        &start,
        &(start + duration),
        &start,
        &true,
        &true,
        &true,
        &None,
    );
    assert_eq!(h.get(id).deposited, boundary);
    assert_eq!(f.balance(&h.contract_id), boundary);
    f.h.assert_pool_exact_for(&f.token);

    // One second before maturity: `deposited * (duration - 1)` is the largest
    // numerator any creation-valid stream can produce, and it must still fit
    // and floor-divide correctly.
    h.warp_to(start + duration - 1);
    let expected = boundary * (d - 1) / d;
    assert_eq!(
        h.client.vested_of(&id),
        expected,
        "the widest representable accrual numerator must not overflow",
    );
    assert_eq!(h.client.withdrawable_of(&id), expected);
    assert_eq!(
        h.client.vested_of(&id) + h.client.refundable_of(&id),
        boundary,
        "conservation just before maturity",
    );

    // At maturity the whole boundary deposit is delivered and the pool drains.
    h.warp_to(start + duration);
    assert_eq!(h.client.vested_of(&id), boundary);
    assert_eq!(h.client.withdraw(&id, &None), boundary);
    assert_eq!(f.balance(&h.recipient), boundary);
    assert_eq!(f.balance(&h.contract_id), 0);
    assert_eq!(h.get(id).status, StreamStatus::Depleted);
    f.h.assert_pool_exact_for(&f.token);
}
