//! Stage 2 — token sub-invocation error categories.
//!
//! Regression suite for [`Error::TokenTransferFailed`] (25) and
//! [`Error::TokenMissing`] (26).
//!
//! ## Design rationale
//!
//! A raw token sub-invocation failure surfaces at the RPC as
//! `Error(Contract, #N)` where `N` is the *token contract's* own discriminant.
//! A client decoding that against Fluxora's error table would misinterpret it
//! silently — e.g. token error #7 reads as `Unauthorized`.
//!
//! Instead, every token failure is mapped onto one of two stable stream-level
//! categories:
//!
//! * `TokenTransferFailed` (25) — token returned a typed contract error
//!   (insufficient balance, pool underfunded, etc.).
//! * `TokenMissing` (26) — host raised an `Abort` (non-contract trap).
//!   On a real network this fires when the token address has no deployed code.
//!   In the test host all sub-invocation failures are contract-typed, so this
//!   variant is verified through its discriminant value only.
//!
//! ## Soroban rollback semantics
//!
//! When a Soroban contract call returns an `Err`, the host rolls back **all**
//! storage writes made during that invocation — including writes made before
//! the failing transfer (the checks-effects-interactions ordering provides
//! reentrancy safety, not persistence-on-failure semantics).
//!
//! Consequences tested here:
//! * A failed `create_stream` leaves no stream entry and does not advance the
//!   id counter.
//! * A failed `withdraw` or `cancel` rolls back accounting and status writes
//!   entirely; the stream is exactly as it was before the call.
//!
//! ## Test matrix
//!
//! | site | scenario | expected error | state after |
//! |---|---|---|---|
//! | `create_stream` | token contract panics → `TokenTransferFailed`* | `TokenTransferFailed` | no entry, id counter unchanged |
//! | `create_stream` | sender balance zero | `TokenTransferFailed` | no entry, id counter unchanged |
//! | `top_up` | sender balance drained | `TokenTransferFailed` | stream unchanged |
//! | `cancel` | pool drained | `TokenTransferFailed` | stream status unchanged (Active) |
//! | `withdraw` | pool drained | `TokenTransferFailed` | withdrawn unchanged, recipient 0 |
//! | `batch_withdraw` | pool drained | `TokenTransferFailed` | recipient 0 |
//! | discriminants | ABI table values | 25 and 26 confirmed | — |
//!
//! *The test host does not distinguish a panicking sub-contract from a
//!  contract-error sub-contract — both surface as `TokenTransferFailed`.
//!  `TokenMissing` is only reachable via WASM execution on a real network.
//!  The variant's discriminant (26) is verified by `token_error_discriminants_match_the_abi_table`.
//!
//! ## Token assumptions — see `docs/ABI.md` "Token assumptions"
//!
//! The rest of this module covers the three assumptions Fluxora states about
//! its token: no fee-on-transfer, no rebasing, and no zero-value transfers.
//!
//! | assumption | site | scenario | expected outcome |
//! |---|---|---|---|
//! | no fee-on-transfer | `create_stream` | deposit pull delivers 90% of `deposit` | `TokenAmountMismatch`, no entry |
//! | no fee-on-transfer | `top_up` | pull delivers 90% of `amount` | `TokenAmountMismatch`, stream unchanged |
//! | no rebasing | `withdraw` | pool balance reduced out-of-band (clawback, standing in for a negative rebase) | `TokenTransferFailed` — fails closed, other streams' accounting untouched |
//! | zero transfers never issued | `cancel` | refund is exactly zero at maturity | succeeds; a token that panics on a zero-value transfer proves none was called |
//! | zero transfers never issued | `withdraw` | nothing vested yet | `NothingToWithdraw`, no token call |
//! | zero transfers never issued | `batch_withdraw` | one stream in the batch has nothing available | batch succeeds; skipped stream's `withdrawn` stays 0 |

use soroban_sdk::testutils::{Address as _, IssuerFlags};
use soroban_sdk::token::{StellarAssetClient, TokenClient};
use soroban_sdk::{contract, contractimpl, symbol_short, Address, Env, MuxedAddress, String};

use super::common::*;
use crate::{Error, StreamStatus};

// ─── panic token ─────────────────────────────────────────────────────────────

/// A minimal token contract whose `transfer` always panics.
///
/// In WASM execution on a real network a `panic!` produces
/// `ScErrorType::WasmVm`, which the host converts to `InvokeError::Abort` and
/// the stream contract surfaces as [`Error::TokenMissing`].
///
/// In the test host contracts execute as native Rust; a `panic!` is caught and
/// converted to a `ScErrorType::Contract` error, which maps to
/// `InvokeError::Contract(_)` and therefore [`Error::TokenTransferFailed`].
/// This is a known test-host limitation — the two paths converge to the same
/// observable client behaviour (a stable, typed stream-level error) even though
/// they use different discriminants in the two environments.
#[contract]
pub struct PanicToken;

#[contractimpl]
impl PanicToken {
    pub fn transfer(_env: Env, _from: Address, _to: MuxedAddress, _amount: i128) {
        panic!("PanicToken: transfer always fails");
    }

    pub fn balance(_env: Env, _id: Address) -> i128 {
        0
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
        String::from_str(&env, "PanicToken")
    }
    pub fn symbol(env: Env) -> String {
        String::from_str(&env, "PANIC")
    }
}

fn register_panic_token(h: &Harness) -> Address {
    h.env.register(PanicToken, ())
}

// ─── clawback-enabled SAC ────────────────────────────────────────────────────

/// Create a fresh Stellar Asset Contract with `ClawbackEnabledFlag` set.
///
/// A dedicated asset per test avoids mutating the harness's main token.
fn make_clawback_token<'a>(
    h: &'a Harness<'a>,
) -> (Address, TokenClient<'a>, StellarAssetClient<'a>) {
    let admin = Address::generate(&h.env);
    let asset = h.env.register_stellar_asset_contract_v2(admin.clone());
    asset.issuer().set_flag(IssuerFlags::ClawbackEnabledFlag);
    let token = asset.address();
    (
        token.clone(),
        TokenClient::new(&h.env, &token),
        StellarAssetClient::new(&h.env, &token),
    )
}

// ─── create_stream ──────────────────────────────────────────────────────────

/// A panicking token causes the sub-invocation to fail with a contract-level
/// error in the test host, surfacing as [`Error::TokenTransferFailed`].
/// (On a real WASM network, the same panic produces `InvokeError::Abort` →
/// [`Error::TokenMissing`] — see module-level notes.)
#[test]
fn create_stream_with_panicking_token_returns_a_token_error() {
    let h = Harness::new();
    let panic_token = register_panic_token(&h);

    let start = h.now();
    let err = h
        .client
        .try_create_stream(
            &h.sender,
            &h.recipient,
            &panic_token,
            &(1_000 * ONE),
            &start,
            &(start + 100 * DAY),
            &start,
            &true,
            &true,
            &true,
        )
        .unwrap_err()
        .unwrap();

    // In the test host this surfaces as TokenTransferFailed; on a WASM network
    // it would be TokenMissing.  Either way it is a stable stream-level error.
    assert!(
        matches!(err, Error::TokenTransferFailed | Error::TokenMissing),
        "expected a token error, got {err:?}"
    );
}

/// A failed `create_stream` must leave no observable stream entry and must not
/// advance the id counter.  Soroban rolls back all storage writes on error.
#[test]
fn create_stream_token_failure_leaves_no_phantom_entry() {
    let h = Harness::new();
    let panic_token = register_panic_token(&h);

    assert_eq!(h.client.stream_count(), 0);

    let start = h.now();
    let _ = h.client.try_create_stream(
        &h.sender,
        &h.recipient,
        &panic_token,
        &(1_000 * ONE),
        &start,
        &(start + 100 * DAY),
        &start,
        &true,
        &true,
        &true,
    );

    // Soroban rolls back the entire invocation on error, so the id counter and
    // any stream entry written before the transfer are both undone.
    assert_eq!(h.client.stream_count(), 0, "id counter must not advance");
    assert!(!h.client.stream_exists(&0), "no phantom entry at id 0");

    // The next create succeeds and gets id 0 (not 1, because the failure
    // rolled back the counter).
    let next_id = h.create_simple(100 * ONE, 10 * DAY);
    assert_eq!(next_id, 0, "successful create must get id 0");
    assert!(h.client.stream_exists(&0));
}

/// When the sender has insufficient balance the SAC returns a typed contract
/// error; the stream contract surfaces it as [`Error::TokenTransferFailed`].
/// No entry is written because the host rolls back on error.
#[test]
fn create_stream_with_sender_insufficient_balance_returns_token_transfer_failed() {
    let h = Harness::new();
    let (token, tc, admin) = make_clawback_token(&h);

    // Mint then immediately drain so the deposit pull will fail.
    admin.mint(&h.sender, &(100 * ONE));
    admin.clawback(&h.sender, &(100 * ONE));
    assert_eq!(tc.balance(&h.sender), 0);

    let start = h.now();
    let err = h
        .client
        .try_create_stream(
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
        )
        .unwrap_err()
        .unwrap();

    assert_eq!(err, Error::TokenTransferFailed);
    assert_eq!(h.client.stream_count(), 0, "id counter must not advance");
    assert!(!h.client.stream_exists(&0));
}

// ─── top_up ─────────────────────────────────────────────────────────────────

/// `top_up` must return [`Error::TokenTransferFailed`] when the sender has no
/// balance.  Because the whole invocation rolls back, the stream's `deposited`
/// and `end_time` must be exactly as they were before the call.
#[test]
fn top_up_returns_token_transfer_failed_when_sender_has_no_balance() {
    let h = Harness::new();
    let (token, tc, admin) = make_clawback_token(&h);

    admin.mint(&h.sender, &(2_000 * ONE));
    let start = h.now();
    let id = h.client.create_stream(
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
    );
    h.advance(10 * DAY);

    let before = h.client.get_stream(&id);

    // Drain sender's remaining balance.
    let remaining = tc.balance(&h.sender);
    if remaining > 0 {
        admin.clawback(&h.sender, &remaining);
    }
    assert_eq!(tc.balance(&h.sender), 0);

    let err = h.client.try_top_up(&id, &(200 * ONE)).unwrap_err().unwrap();

    assert_eq!(err, Error::TokenTransferFailed);

    // Rollback: stream must be exactly as before.
    let after = h.client.get_stream(&id);
    assert_eq!(after.deposited, before.deposited);
    assert_eq!(after.end_time, before.end_time);

    // Confirm a subsequent top_up works once the balance is restored.
    admin.mint(&h.sender, &(500 * ONE));
    h.client.top_up(&id, &(200 * ONE));
    assert_eq!(h.client.get_stream(&id).deposited, 1_200 * ONE);
}

// ─── cancel ─────────────────────────────────────────────────────────────────

/// `cancel` must return [`Error::TokenTransferFailed`] when the pool is empty.
/// Because Soroban rolls back all writes on error, the stream status stays
/// `Active` — the cancel did not take effect.
#[test]
fn cancel_returns_token_transfer_failed_when_pool_is_underfunded() {
    let h = Harness::new();
    let (token, tc, admin) = make_clawback_token(&h);
    let contract_id = h.contract_id.clone();

    admin.mint(&h.sender, &(1_000 * ONE));
    let start = h.now();
    let id = h.client.create_stream(
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
    );
    h.advance(10 * DAY);

    assert!(h.client.refundable_of(&id) > 0);

    // Drain the pool so the refund transfer will fail.
    let pool = tc.balance(&contract_id);
    admin.clawback(&contract_id, &pool);

    let err = h.client.try_cancel(&id).unwrap_err().unwrap();
    assert_eq!(err, Error::TokenTransferFailed);

    // Rollback: stream status must still be Active, nothing moved.
    assert_eq!(
        h.client.get_stream(&id).status,
        StreamStatus::Active,
        "cancel rolled back entirely; stream must still be Active"
    );
    assert_eq!(tc.balance(&h.sender), 0, "no refund moved");
}

// ─── withdraw / apply_withdrawal ────────────────────────────────────────────

/// `withdraw` must return [`Error::TokenTransferFailed`] when the pool is
/// empty.  Rollback means `withdrawn` is unchanged and the recipient has zero.
#[test]
fn withdraw_returns_token_transfer_failed_when_pool_is_underfunded() {
    let h = Harness::new();
    let (token, tc, admin) = make_clawback_token(&h);
    let contract_id = h.contract_id.clone();

    admin.mint(&h.sender, &(1_000 * ONE));
    let start = h.now();
    let id = h.client.create_stream(
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
    );
    h.advance(50 * DAY);

    assert!(h.client.withdrawable_of(&id) > 0);

    let pool = tc.balance(&contract_id);
    admin.clawback(&contract_id, &pool);

    let err = h.client.try_withdraw(&id, &None).unwrap_err().unwrap();
    assert_eq!(err, Error::TokenTransferFailed);

    // Rollback: nothing changed.
    assert_eq!(tc.balance(&h.recipient), 0);
    assert_eq!(h.client.get_stream(&id).withdrawn, 0);
}

/// `batch_withdraw` must propagate [`Error::TokenTransferFailed`] when the
/// pool is drained.
#[test]
fn batch_withdraw_returns_token_transfer_failed_when_pool_is_underfunded() {
    let h = Harness::new();
    let (token, tc, admin) = make_clawback_token(&h);
    let contract_id = h.contract_id.clone();

    admin.mint(&h.sender, &(2_000 * ONE));
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
    );
    h.advance(50 * DAY);

    let pool = tc.balance(&contract_id);
    admin.clawback(&contract_id, &pool);

    let err = h
        .client
        .try_batch_withdraw(&h.recipient, &h.ids(&[a, b]))
        .unwrap_err()
        .unwrap();

    assert_eq!(err, Error::TokenTransferFailed);
    assert_eq!(tc.balance(&h.recipient), 0);
}

// ─── boundary / retry ────────────────────────────────────────────────────────

/// After a `TokenTransferFailed` on `withdraw`, Soroban rolls back all writes
/// including the `withdrawn` counter increment.  Replenishing the pool and
/// retrying succeeds and pays out the full amount — the failed call left no
/// trace in the accounting.
#[test]
fn withdraw_is_retryable_once_pool_is_replenished() {
    let h = Harness::new();
    let (token, tc, admin) = make_clawback_token(&h);
    let contract_id = h.contract_id.clone();

    admin.mint(&h.sender, &(1_000 * ONE));
    let start = h.now();
    let id = h.client.create_stream(
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
    );
    h.advance(50 * DAY);

    let available = h.client.withdrawable_of(&id);

    // Drain, attempt, confirm failure.
    let pool = tc.balance(&contract_id);
    admin.clawback(&contract_id, &pool);
    let err = h.client.try_withdraw(&id, &None).unwrap_err().unwrap();
    assert_eq!(err, Error::TokenTransferFailed);
    assert_eq!(tc.balance(&h.recipient), 0);

    // Replenish and retry.  Because the failed call rolled back entirely,
    // the retry sees the full withdrawable balance and succeeds.
    admin.mint(&contract_id, &available);
    let paid = h.client.withdraw(&id, &None);
    assert_eq!(paid, available);
    assert_eq!(tc.balance(&h.recipient), available);
}

// ─── fee-on-transfer token ───────────────────────────────────────────────────

/// A token whose `transfer` debits `from` in full but credits `to` only
/// `amount` minus a configurable fee — the fee simply vanishes, the same
/// observable effect as a burn-on-transfer or reflective token.
///
/// The fee is off (0 bps) by default, so a stream can be created normally;
/// [`FeeOnTransferTokenClient::set_fee_bps`] turns it on so a test can isolate
/// exactly which call (`create_stream` vs. `top_up`) is expected to detect
/// and reject the shortfall.
///
/// Balances are tracked in this contract's own instance storage — a
/// test-only stand-in for a real SEP-41 ledger, not a SAC wrapper, so the fee
/// can be applied deterministically on every `transfer`.
#[contract]
pub struct FeeOnTransferToken;

#[contractimpl]
impl FeeOnTransferToken {
    /// Test-only mint, bypassing transfer/fee semantics entirely.
    pub fn mint(env: Env, to: Address, amount: i128) {
        let bal = Self::balance_of(&env, &to);
        env.storage().instance().set(&to, &(bal + amount));
    }

    /// Test-only: set the fee, in basis points of the transferred amount.
    pub fn set_fee_bps(env: Env, bps: u32) {
        env.storage()
            .instance()
            .set(&symbol_short!("fee_bps"), &bps);
    }

    fn fee_bps(env: &Env) -> u32 {
        env.storage()
            .instance()
            .get(&symbol_short!("fee_bps"))
            .unwrap_or(0)
    }

    fn balance_of(env: &Env, id: &Address) -> i128 {
        env.storage().instance().get(id).unwrap_or(0)
    }

    pub fn transfer(env: Env, from: Address, to: MuxedAddress, amount: i128) {
        let to = to.address();
        let from_bal = Self::balance_of(&env, &from);
        assert!(
            from_bal >= amount,
            "FeeOnTransferToken: insufficient balance"
        );
        env.storage().instance().set(&from, &(from_bal - amount));

        let bps = Self::fee_bps(&env) as i128;
        let fee = amount * bps / 10_000;
        let credited = amount - fee;
        let to_bal = Self::balance_of(&env, &to);
        env.storage().instance().set(&to, &(to_bal + credited));
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
        String::from_str(&env, "FeeOnTransferToken")
    }
    pub fn symbol(env: Env) -> String {
        String::from_str(&env, "FEE")
    }
}

/// Register a fee-on-transfer token with the fee off, fund `sender`, and
/// return `(token, client)`.
fn register_fee_on_transfer_token<'a>(
    h: &'a Harness<'a>,
) -> (Address, FeeOnTransferTokenClient<'a>) {
    let token = h.env.register(FeeOnTransferToken, ());
    let client = FeeOnTransferTokenClient::new(&h.env, &token);
    client.mint(&h.sender, &(10_000 * ONE));
    (token, client)
}

/// `create_stream`'s deposit pull must deliver exactly `deposit`. A
/// fee-on-transfer token delivers less, so the pull is detected and rejected
/// with [`Error::TokenAmountMismatch`] rather than silently under-collateralizing
/// the stream. See `docs/ABI.md` "Token assumptions" #1.
#[test]
fn create_stream_with_fee_on_transfer_token_is_rejected() {
    let h = Harness::new();
    let (token, fee_token) = register_fee_on_transfer_token(&h);
    fee_token.set_fee_bps(&1_000); // 10%

    let start = h.now();
    let err = h
        .client
        .try_create_stream(
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
        )
        .unwrap_err()
        .unwrap();

    assert_eq!(err, Error::TokenAmountMismatch);

    // Rollback is total: no phantom entry, id counter untouched, and the
    // already-executed (fee-taking) transfer is undone along with it — the
    // sender is made whole, fee included.
    assert_eq!(h.client.stream_count(), 0, "id counter must not advance");
    assert!(!h.client.stream_exists(&0));
    assert_eq!(
        fee_token.balance(&h.sender),
        10_000 * ONE,
        "reverted pull must not cost the sender the fee"
    );
}

/// Same detection on `top_up`: the fee is turned on only *after* the stream
/// is created with the fee off, isolating the `top_up` pull as the call under
/// test. The schedule and deposit must be exactly as they were before the
/// rejected call.
#[test]
fn top_up_with_fee_on_transfer_token_is_rejected() {
    let h = Harness::new();
    let (token, fee_token) = register_fee_on_transfer_token(&h);

    let start = h.now();
    let id = h.client.create_stream(
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
    );
    h.advance(10 * DAY);
    let before = h.client.get_stream(&id);

    fee_token.set_fee_bps(&1_000); // 10%, turned on after creation
    let err = h.client.try_top_up(&id, &(200 * ONE)).unwrap_err().unwrap();
    assert_eq!(err, Error::TokenAmountMismatch);

    let after = h.client.get_stream(&id);
    assert_eq!(
        after.deposited, before.deposited,
        "rejected top-up must not add funds"
    );
    assert_eq!(
        after.end_time, before.end_time,
        "rejected top-up must not extend the schedule"
    );
}

// ─── rebasing / out-of-band balance loss ────────────────────────────────────

/// A rebase is not a `transfer` Fluxora is party to, so it cannot be detected
/// at call time — there is nothing to instrument. `admin.clawback` on the
/// pool directly (bypassing every Fluxora entry point) stands in for the
/// out-of-band balance loss a negative rebase would cause. The documented
/// contract is that this fails *closed*: the underfunded stream's `withdraw`
/// returns [`Error::TokenTransferFailed`] rather than paying a wrong amount,
/// and an unrelated stream sharing the same token is untouched. See
/// `docs/ABI.md` "Token assumptions" #2.
#[test]
fn rebase_style_balance_loss_fails_closed_and_does_not_corrupt_other_streams() {
    let h = Harness::new();
    let (token, tc, admin) = make_clawback_token(&h);
    let contract_id = h.contract_id.clone();

    admin.mint(&h.sender, &(2_000 * ONE));
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
    );
    h.advance(50 * DAY);

    let before_b = h.client.get_stream(&b);

    // Out-of-band balance loss on the pool — stands in for a negative
    // rebase. Leaves just enough to cover stream A alone, well short of both
    // streams' combined outstanding liability.
    let a_liability = h.client.withdrawable_of(&a);
    admin.clawback(&contract_id, &(tc.balance(&contract_id) - a_liability));

    // A drains what is left; B — untouched by any Fluxora call — must find
    // the pool empty and fail closed rather than paying a partial or wrong
    // amount.
    h.client.withdraw(&a, &None);
    let err = h.client.try_withdraw(&b, &None).unwrap_err().unwrap();
    assert_eq!(err, Error::TokenTransferFailed);

    // B's own accounting is untouched by A's withdrawal or by the
    // out-of-band loss: the rebase corrupted the pool's real balance, not
    // Fluxora's bookkeeping.
    let after_b = h.client.get_stream(&b);
    assert_eq!(after_b.withdrawn, before_b.withdrawn);
    assert_eq!(after_b.deposited, before_b.deposited);
    assert_eq!(after_b.status, StreamStatus::Active);
}

// ─── zero-value transfers are never issued ──────────────────────────────────

/// A token that behaves normally for any nonzero `transfer` but panics on a
/// zero-value one. Used to prove Fluxora never issues a zero-value transfer:
/// if it did, this token would panic and the call would fail loudly instead
/// of returning the typed error (or succeeding) the test expects.
#[contract]
pub struct ZeroGuardToken;

#[contractimpl]
impl ZeroGuardToken {
    pub fn mint(env: Env, to: Address, amount: i128) {
        let bal = Self::balance_of(&env, &to);
        env.storage().instance().set(&to, &(bal + amount));
    }

    fn balance_of(env: &Env, id: &Address) -> i128 {
        env.storage().instance().get(id).unwrap_or(0)
    }

    pub fn transfer(env: Env, from: Address, to: MuxedAddress, amount: i128) {
        assert_ne!(amount, 0, "ZeroGuardToken: unexpected zero-value transfer");
        let to = to.address();
        let from_bal = Self::balance_of(&env, &from);
        assert!(from_bal >= amount, "ZeroGuardToken: insufficient balance");
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
        String::from_str(&env, "ZeroGuardToken")
    }
    pub fn symbol(env: Env) -> String {
        String::from_str(&env, "ZG")
    }
}

fn register_zero_guard_token<'a>(h: &'a Harness<'a>) -> (Address, ZeroGuardTokenClient<'a>) {
    let token = h.env.register(ZeroGuardToken, ());
    let client = ZeroGuardTokenClient::new(&h.env, &token);
    client.mint(&h.sender, &(10_000 * ONE));
    (token, client)
}

/// `cancel` skips the refund transfer entirely when `refund == 0` (e.g.
/// cancelling an already fully-vested stream). Proven here with a token that
/// panics on any zero-value transfer: if `cancel` ever called it with zero,
/// this test would fail with a panic instead of a clean `Cancelled` status.
#[test]
fn cancel_with_zero_refund_never_calls_transfer() {
    let h = Harness::new();
    let (token, _zg) = register_zero_guard_token(&h);

    let start = h.now();
    let id = h.client.create_stream(
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
    );
    // Fully matured: everything is vested, nothing is left to refund.
    h.advance(100 * DAY);
    assert_eq!(h.client.refundable_of(&id), 0);

    h.client.cancel(&id);

    assert_eq!(h.client.get_stream(&id).status, StreamStatus::Cancelled);
    assert_eq!(h.client.get_stream(&id).deposited, 1_000 * ONE);
}

/// `withdraw` on a stream with nothing vested yet returns
/// [`Error::NothingToWithdraw`] before ever touching the token. Proven with a
/// token that panics on a zero-value transfer.
#[test]
fn withdraw_with_nothing_vested_never_calls_transfer() {
    let h = Harness::new();
    let (token, _zg) = register_zero_guard_token(&h);

    let start = h.now();
    let id = h.client.create_stream(
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
    );
    // No time has passed: nothing is vested yet.
    let err = h.client.try_withdraw(&id, &None).unwrap_err().unwrap();
    assert_eq!(err, Error::NothingToWithdraw);
}

/// `batch_withdraw` skips a stream with nothing currently available rather
/// than paying it a zero. Proven with a token that panics on a zero-value
/// transfer: a mixed batch (one stream with nothing available, one with a
/// real payout) must succeed without the skipped stream ever reaching the
/// token.
#[test]
fn batch_withdraw_skips_zero_available_stream_without_calling_transfer() {
    let h = Harness::new();
    let (token, _zg) = register_zero_guard_token(&h);

    let start = h.now();
    // `behind` has a cliff well past the batch's advance point, so it has
    // nothing withdrawable yet.
    let behind = h.client.create_stream(
        &h.sender,
        &h.recipient,
        &token,
        &(1_000 * ONE),
        &start,
        &(start + 100 * DAY),
        &(start + 50 * DAY),
        &true,
        &true,
        &true,
    );
    let ready = h.client.create_stream(
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
    );
    h.advance(10 * DAY);
    assert_eq!(h.client.withdrawable_of(&behind), 0);
    assert!(h.client.withdrawable_of(&ready) > 0);

    let total = h
        .client
        .batch_withdraw(&h.recipient, &h.ids(&[behind, ready]));

    assert!(total > 0);
    assert_eq!(h.client.get_stream(&behind).withdrawn, 0);
    assert!(h.client.get_stream(&ready).withdrawn > 0);
}

// ─── discriminants ───────────────────────────────────────────────────────────

/// Confirm the frozen ABI discriminants for both token error variants.
#[test]
fn token_error_discriminants_match_the_abi_table() {
    assert_eq!(Error::TokenTransferFailed as u32, 25);
    assert_eq!(Error::TokenMissing as u32, 26);
}

/// Confirm the frozen ABI discriminant for the fee-on-transfer / rebase
/// detection error.
#[test]
fn token_amount_mismatch_discriminant_matches_the_abi_table() {
    assert_eq!(Error::TokenAmountMismatch as u32, 32);
}
