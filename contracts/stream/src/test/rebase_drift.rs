//! Issue #1805 — detect a rebasing token that desynchronises the pool.
//!
//! `docs/KNOWN-LIMITATIONS.md` §6 recorded that an elastic-supply token could
//! change the pool's real balance between two operations without the contract
//! noticing: the deposit leg measures a balance *delta* (so it catches
//! fee-on-transfer), but nothing re-read the pool afterwards. The pool total
//! Fluxora now maintains per token in [`DataKey::PooledBalance`] closes that
//! hole — every funds-moving operation ends by reconciling the total against
//! the token's own `balance`, and a shortfall is rejected with
//! [`Error::PoolBalanceDrift`].
//!
//! # The fixture
//!
//! [`RebasingToken`] is a SEP-41-shaped token whose balances live in its own
//! instance storage, plus one test-only entry point — [`rebase_to`] — that
//! overwrites a balance *without* any transfer. That is precisely what an
//! elastic-supply rebase does, and it is the only way to arrange the failure
//! this issue is about: a balance change Fluxora is not a party to, so there is
//! no transfer to instrument.
//!
//! Every test here drives the contract through its public ABI; the rebase is
//! the only thing injected out-of-band, exactly as it would be on chain.
//!
//! [`DataKey::PooledBalance`]: crate::DataKey::PooledBalance
//! [`rebase_to`]: RebasingTokenClient::rebase_to

use soroban_sdk::token::TokenClient;
use soroban_sdk::{contract, contractimpl, Address, Env, MuxedAddress, String};

use super::common::*;
use crate::{Error, StreamStatus};

// ─── rebasing token fixture ──────────────────────────────────────────────────

/// A token whose balances change outside transfers.
///
/// Balances are tracked in this contract's own instance storage — a test-only
/// stand-in for a real SEP-41 ledger — so a test can move a balance with no
/// corresponding `transfer`, which is what makes a rebase undetectable by
/// instrumenting transfers alone.
#[contract]
pub struct RebasingToken;

#[contractimpl]
impl RebasingToken {
    /// Test-only mint, bypassing transfer semantics entirely.
    pub fn mint(env: Env, to: Address, amount: i128) {
        let bal = Self::balance_of(&env, &to);
        env.storage().instance().set(&to, &(bal + amount));
    }

    /// Test-only **rebase**: overwrite `id`'s balance outright.
    ///
    /// No transfer, no event, no counterparty — the supply simply changes.
    /// This is the out-of-band balance move the contract has to detect.
    pub fn rebase_to(env: Env, id: Address, amount: i128) {
        env.storage().instance().set(&id, &amount);
    }

    fn balance_of(env: &Env, id: &Address) -> i128 {
        env.storage().instance().get(id).unwrap_or(0)
    }

    /// A normal transfer: moves exactly `amount`, in full, both sides.
    pub fn transfer(env: Env, from: Address, to: MuxedAddress, amount: i128) {
        let to = to.address();
        let from_bal = Self::balance_of(&env, &from);
        assert!(
            from_bal >= amount,
            "RebasingToken: insufficient balance for transfer"
        );
        env.storage().instance().set(&from, &(from_bal - amount));
        let to_bal = Self::balance_of(&env, &to);
        env.storage().instance().set(&to, &(to_bal + amount));
    }

    pub fn balance(env: Env, id: Address) -> i128 {
        Self::balance_of(&env, &id)
    }

    pub fn allowance(_env: Env, _from: Address, _spender: Address) -> i128 {
        0
    }
    pub fn approve(
        _env: Env,
        _from: Address,
        _spender: Address,
        _amount: i128,
        _live_until_ledger: u32,
    ) {
    }
    pub fn transfer_from(
        _env: Env,
        _spender: Address,
        _from: Address,
        _to: Address,
        _amount: i128,
    ) {
    }
    pub fn burn(_env: Env, _from: Address, _amount: i128) {}
    pub fn burn_from(_env: Env, _spender: Address, _from: Address, _amount: i128) {}
    pub fn decimals(_env: Env) -> u32 {
        7
    }
    pub fn name(env: Env) -> String {
        String::from_str(&env, "RebasingToken")
    }
    pub fn symbol(env: Env) -> String {
        String::from_str(&env, "REBASE")
    }
}

/// Register a rebasing token, fund `sender`, and return `(token, client)`.
pub(super) fn register_rebasing_token<'a, 'b>(
    h: &'a Harness<'b>,
) -> (Address, RebasingTokenClient<'a>) {
    let token = h.env.register(RebasingToken, ());
    let client = RebasingTokenClient::new(&h.env, &token);
    client.mint(&h.sender, &(10_000 * ONE));
    (token, client)
}

/// Deposit `deposit` of a fresh rebasing token into a new 100-day stream.
///
/// Shared by every test below: one stream, one token, nothing else in the pool.
fn stream_on_rebasing_token<'a, 'b>(
    h: &'a Harness<'b>,
    deposit: i128,
) -> (Address, RebasingTokenClient<'a>, u64) {
    let (token, rebasing) = register_rebasing_token(h);
    let start = h.now();
    let id = h.client.create_stream(
        &h.sender,
        &h.recipient,
        &token,
        &deposit,
        &start,
        &(start + 100 * DAY),
        &start,
        &true,
        &true,
        &true,
        &None,
    );
    (token, rebasing, id)
}

/// Drive a public entry point into [`Error::PoolBalanceDrift`], for
/// `test::error_reachability`'s `Reach` classification.
///
/// Deposit, let a third of the schedule vest, rebase the pool down out of band,
/// then withdraw: the payout is still coverable, so the only error the
/// withdrawal can produce is the reconciliation failure this module is about.
pub(super) fn drift_error(h: &Harness) -> Error {
    let (_token, rebasing, id) = stream_on_rebasing_token(h, 1_000 * ONE);
    h.advance(30 * DAY);
    rebasing.rebase_to(&h.contract_id, &(600 * ONE));
    h.client.try_withdraw(&id, &None).unwrap_err().unwrap()
}

// ─── the acceptance case: a rebase between deposit and withdrawal ────────────

/// The issue's own reproduction: a rebase lands between the deposit and the
/// withdrawal. The withdrawal must be detected and refused — a named error and
/// a full rollback — rather than silently paying a recipient out of another
/// stream's claim.
#[test]
fn a_rebase_between_deposit_and_withdrawal_is_detected() {
    let h = Harness::new();
    let (_token, rebasing, id) = stream_on_rebasing_token(&h, 1_000 * ONE);
    h.advance(30 * DAY);

    // Pre-conditions: the pool really holds the whole deposit, and a third of
    // the schedule has vested.
    assert_eq!(rebasing.balance(&h.contract_id), 1_000 * ONE);
    let available = h.client.withdrawable_of(&id);
    assert_eq!(available, 300 * ONE);

    // The rebase: 400 ONE of supply disappears from the pool. No transfer is
    // involved, so the deposit-side delta check cannot see it.
    rebasing.rebase_to(&h.contract_id, &(600 * ONE));

    let err = h.client.try_withdraw(&id, &None).unwrap_err().unwrap();
    assert_eq!(
        err,
        Error::PoolBalanceDrift,
        "a rebase between deposit and withdrawal must be detected",
    );

    // The whole invocation rolled back: no tokens moved, no accounting changed.
    // (The payout was still coverable at 600 ONE — the drift, not an
    // insufficient balance, is what rejected the call.)
    assert_eq!(rebasing.balance(&h.recipient), 0, "no payout may move");
    assert_eq!(rebasing.balance(&h.contract_id), 600 * ONE);
    let stream = h.get(id);
    assert_eq!(stream.withdrawn, 0, "withdrawn counter must not advance");
    assert_eq!(stream.status, StreamStatus::Active);
}

/// The drift is caught even though the token handed out the payout correctly —
/// a well-behaved transfer on a pool that is short is still misaccounting.
#[test]
fn an_explicit_partial_withdrawal_is_detected_too() {
    let h = Harness::new();
    let (_token, rebasing, id) = stream_on_rebasing_token(&h, 1_000 * ONE);
    h.advance(30 * DAY);

    // Short by 100 ONE only — far less than the 300 ONE payout, so nothing
    // about the transfer itself looks wrong.
    rebasing.rebase_to(&h.contract_id, &(900 * ONE));

    let err = h
        .client
        .try_withdraw(&id, &Some(100 * ONE))
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::PoolBalanceDrift);
    assert_eq!(rebasing.balance(&h.recipient), 0);
}

// ─── the other two operations that move pool funds ───────────────────────────

/// A rebase is also detected on the way *in*: `top_up` reconciles the token's
/// pool total after its pull, so a stream cannot be extended on top of a pool
/// that already lost balance.
#[test]
fn a_rebase_between_deposit_and_top_up_is_detected() {
    let h = Harness::new();
    let (_token, rebasing, id) = stream_on_rebasing_token(&h, 1_000 * ONE);
    h.advance(30 * DAY);

    rebasing.rebase_to(&h.contract_id, &(600 * ONE));

    let err = h.client.try_top_up(&id, &(100 * ONE)).unwrap_err().unwrap();
    assert_eq!(err, Error::PoolBalanceDrift);

    // Rolled back: the sender kept their tokens and the schedule is unchanged.
    assert_eq!(rebasing.balance(&h.sender), 9_000 * ONE);
    assert_eq!(h.get(id).deposited, 1_000 * ONE);
}

/// `cancel` refunds the unvested remainder, so it is the third way tokens leave
/// the pool — and the third place a rebase has to be caught. The refund here is
/// small enough to be coverable by the rebased pool, so the rejection can only
/// come from the reconciliation.
#[test]
fn a_rebase_between_deposit_and_cancel_is_detected() {
    let h = Harness::new();
    let (_token, rebasing, id) = stream_on_rebasing_token(&h, 1_000 * ONE);

    // 90 of 100 days vested: the refund is 100 ONE, comfortably covered by the
    // 500 ONE left after the rebase.
    h.advance(90 * DAY);
    assert_eq!(h.client.refundable_of(&id), 100 * ONE);
    rebasing.rebase_to(&h.contract_id, &(500 * ONE));

    let err = h.client.try_cancel(&id).unwrap_err().unwrap();
    assert_eq!(err, Error::PoolBalanceDrift);

    // Nothing was refunded and the stream is still active.
    assert_eq!(rebasing.balance(&h.sender), 9_000 * ONE);
    assert_eq!(h.get(id).status, StreamStatus::Active);
}

/// `batch_withdraw` reconciles each token in the batch once, after every payout
/// has landed. A rebase on a token two streams share is caught once, and the
/// whole batch reverts.
#[test]
fn a_rebase_is_detected_by_batch_withdraw() {
    let h = Harness::new();
    let (token, rebasing) = register_rebasing_token(&h);
    let start = h.now();

    let a = h.client.create_stream(
        &h.sender,
        &h.recipient,
        &token,
        &(1_000 * ONE),
        &start,
        &(start + 100 * DAY),
        &start,
        &true,
        &true,
        &true,
        &None,
    );
    let b = h.client.create_stream(
        &h.sender,
        &h.recipient,
        &token,
        &(1_000 * ONE),
        &start,
        &(start + 100 * DAY),
        &start,
        &true,
        &true,
        &true,
        &None,
    );
    h.advance(50 * DAY);

    // Two 500 ONE payouts are still coverable by the 1_500 ONE left, so again
    // nothing about the transfers looks wrong.
    rebasing.rebase_to(&h.contract_id, &(1_500 * ONE));

    let err = h
        .client
        .try_batch_withdraw(&h.recipient, &h.ids(&[a, b]))
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::PoolBalanceDrift);

    assert_eq!(rebasing.balance(&h.recipient), 0, "batch must roll back");
    assert_eq!(h.get(a).withdrawn, 0);
    assert_eq!(h.get(b).withdrawn, 0);
}

// ─── scope: what is deliberately *not* rejected ──────────────────────────────

/// A **positive** rebase is tolerated, and the surplus is inert.
///
/// Two reasons, both load-bearing:
///
/// 1. A surplus cannot cause an underpayment — every payout is sized by stream
///    accounting, never by the pool balance — so it is not a solvency risk.
/// 2. Demanding exact equality would hand any third party a griefing stick:
///    `transfer` one stroop into the contract and every withdrawal in the
///    protocol reverts. A check anyone can trip is worse than the risk it
///    guards.
#[test]
fn a_positive_rebase_is_tolerated_and_its_surplus_stays_inert() {
    let h = Harness::new();
    let (_token, rebasing, id) = stream_on_rebasing_token(&h, 1_000 * ONE);
    h.advance(30 * DAY);

    // 50% more supply: 1_500 ONE in a pool that accounts for 1_000.
    rebasing.rebase_to(&h.contract_id, &(1_500 * ONE));

    let paid = h.client.withdraw(&id, &None);
    assert_eq!(paid, 300 * ONE, "the surplus must not inflate a payout");
    assert_eq!(rebasing.balance(&h.recipient), 300 * ONE);
    assert_eq!(
        rebasing.balance(&h.contract_id),
        1_200 * ONE,
        "exactly the payout left; the surplus is still there and still unclaimed",
    );

    // And the surplus does not wedge the stream: later operations still work.
    h.advance(30 * DAY);
    let paid_again = h.client.withdraw(&id, &None);
    assert_eq!(paid_again, 300 * ONE);
}

/// Dust is the minimal version of the same case — the check must not be
/// triggerable by a one-unit transfer from an unrelated address.
#[test]
fn a_one_unit_donation_does_not_freeze_withdrawals() {
    let h = Harness::new();
    let (token, rebasing, id) = stream_on_rebasing_token(&h, 1_000 * ONE);
    h.advance(30 * DAY);

    // `other` — not a party to this stream — dusts the pool.
    rebasing.mint(&h.other, &1);
    TokenClient::new(&h.env, &token).transfer(&h.other, &h.contract_id, &1);
    assert_eq!(rebasing.balance(&h.contract_id), 1_000 * ONE + 1);

    let paid = h.client.withdraw(&id, &None);
    assert_eq!(paid, 300 * ONE, "a donation must not block a withdrawal");
}

// ─── per-token isolation ─────────────────────────────────────────────────────

/// The pool total is per token, not global: a rebase on one token must not
/// block withdrawals from a stream funded with a healthy one.
#[test]
fn a_drift_on_one_token_does_not_block_a_healthy_token() {
    let h = Harness::new();
    let (token_bad, bad) = register_rebasing_token(&h);
    let (token_good, good) = register_rebasing_token(&h);
    let start = h.now();

    let bad_stream = h.client.create_stream(
        &h.sender,
        &h.recipient,
        &token_bad,
        &(1_000 * ONE),
        &start,
        &(start + 100 * DAY),
        &start,
        &true,
        &true,
        &true,
        &None,
    );
    let good_stream = h.client.create_stream(
        &h.sender,
        &h.recipient,
        &token_good,
        &(1_000 * ONE),
        &start,
        &(start + 100 * DAY),
        &start,
        &true,
        &true,
        &true,
        &None,
    );
    h.advance(50 * DAY);

    // Only the first token rebases.
    bad.rebase_to(&h.contract_id, &(600 * ONE));

    let err = h
        .client
        .try_withdraw(&bad_stream, &None)
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::PoolBalanceDrift);

    // The healthy token is unaffected: its own bookkeeping still balances.
    let paid = h.client.withdraw(&good_stream, &None);
    assert_eq!(paid, 500 * ONE);
    assert_eq!(good.balance(&h.recipient), 500 * ONE);
    assert_eq!(good.balance(&h.contract_id), 500 * ONE);
    // ...and the drifted pool is exactly as the rebase left it.
    assert_eq!(bad.balance(&h.contract_id), 600 * ONE);
}

// ─── ABI ─────────────────────────────────────────────────────────────────────

/// The variant is appended, never renumbered: 39 is the slot after the
/// contract-level halt block. Also pinned in `test::error_discriminants`.
#[test]
fn pool_balance_drift_discriminant_value() {
    assert_eq!(
        Error::PoolBalanceDrift as u32,
        39,
        "PoolBalanceDrift discriminant must be 39",
    );
}
