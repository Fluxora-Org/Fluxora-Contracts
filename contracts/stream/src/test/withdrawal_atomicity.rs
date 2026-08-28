//! Regression tests for issue #1571 — withdrawal bookkeeping atomicity.
//!
//! # What this tests
//!
//! `apply_withdrawal` updates `stream.withdrawn` (and possibly `stream.status`)
//! and persists the stream *before* calling the token contract's `transfer`.
//! The comment in `apply_withdrawal` explains the ordering choice: it is the
//! standard checks-effects-interactions pattern, and Soroban's host forbids
//! reentrancy outright, so there is no classical reentrancy risk.
//!
//! The correctness argument for atomicity is different: on Soroban, **every
//! host trap propagates as a Rust panic that unwinds the entire transaction**.
//! The Soroban test host replicates this exactly — if a nested contract call
//! panics, control unwinds back to the outermost `invoke_contract` call and
//! all storage writes made in that invocation are discarded.  No committed
//! state leaks out of a failed transaction.
//!
//! These tests *prove* that claim in the context of `withdraw` and
//! `batch_withdraw`.  Each test:
//!
//! 1. Establishes a stream with a non-zero withdrawable balance.
//! 2. Engineers a token `transfer` failure (two mechanisms, see below).
//! 3. Calls `withdraw` / `batch_withdraw` via `catch_unwind` and observes the
//!    panic.
//! 4. Reads stream state and token balances **after** the panic and asserts
//!    they are byte-for-byte identical to what they were before.
//!
//! Nothing is permanently marked withdrawn; no double-pay is possible on retry.
//!
//! # Two failure mechanisms exercised
//!
//! ## 1. SAC authorization revoked (`set_authorized`)
//!
//! The Stellar Asset Contract rejects a `transfer` whose `to` address is not
//! authorized to hold the asset.  We call `token_admin.set_authorized(&recipient,
//! &false)` before the withdrawal.  For this to work the issuer account must
//! have the `AUTH_REVOCABLE_FLAG` (bit 2) set; the `RevocableHarness` below
//! arranges this via `StellarAssetIssuer::set_flag`.
//!
//! ## 2. Always-panicking token contract
//!
//! A minimal contract registered at the same address as the real token (using
//! `env.register_at`, which replaces the existing contract) that always panics
//! in its `transfer` implementation.  This covers a hard host-level trap rather
//! than a soft contract error, and proves that even a maximally hostile token
//! cannot corrupt bookkeeping.

use soroban_sdk::testutils::{Address as _, IssuerFlags, Ledger as _};
use soroban_sdk::token::{Client as TokenClient, StellarAssetClient};
use soroban_sdk::{contract, contractimpl, Address, Env, MuxedAddress};

use super::common::*;
use crate::{FluxoraStream, FluxoraStreamClient, StreamStatus};

// ---------------------------------------------------------------------------
// Failing token contract
// ---------------------------------------------------------------------------

/// A token contract stub whose `transfer` function always panics.
///
/// Registered at the real token's address via `env.register_at` to replace it
/// for the duration of a single test call.  Only `transfer` needs to be
/// implemented because that is the only function `apply_withdrawal` calls.
#[contract]
struct AlwaysPanicsToken;

#[contractimpl]
impl AlwaysPanicsToken {
    #[allow(unused_variables)]
    pub fn transfer(env: Env, from: Address, to: MuxedAddress, amount: i128) {
        panic!("token transfer deliberately rejected by AlwaysPanicsToken");
    }
}

// ---------------------------------------------------------------------------
// Revocable test harness
// ---------------------------------------------------------------------------

/// A variant of the standard test harness that creates the SAC with
/// `AUTH_REVOCABLE_FLAG` set on the issuer account, so that
/// `token_admin.set_authorized(address, false)` works without panicking.
struct RevocableHarness<'a> {
    env: Env,
    client: FluxoraStreamClient<'a>,
    contract_id: Address,
    token: Address,
    token_client: TokenClient<'a>,
    token_admin: StellarAssetClient<'a>,
    sender: Address,
    recipient: Address,
}

impl<'a> RevocableHarness<'a> {
    fn new() -> RevocableHarness<'a> {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(T0);

        let contract_id = env.register(FluxoraStream, ());
        let client = FluxoraStreamClient::new(&env, &contract_id);

        let issuer = Address::generate(&env);
        let asset = env.register_stellar_asset_contract_v2(issuer);

        // Enable AUTH_REVOCABLE_FLAG on the issuer so that set_authorized can
        // be used to deauthorize recipients.
        asset.issuer().set_flag(IssuerFlags::RevocableFlag);

        let token = asset.address();
        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);

        let token_admin = StellarAssetClient::new(&env, &token);
        token_admin.mint(&sender, &(1_000_000 * ONE));

        let token_client = TokenClient::new(&env, &token);

        RevocableHarness {
            client,
            contract_id,
            token: token.clone(),
            token_client,
            token_admin,
            sender,
            recipient,
            env,
        }
    }

    fn advance(&self, seconds: u64) {
        let info = self.env.ledger().get();
        self.env.ledger().set_timestamp(info.timestamp + seconds);
    }

    fn warp_to(&self, ts: u64) {
        self.env.ledger().set_timestamp(ts);
    }

    fn now(&self) -> u64 {
        self.env.ledger().timestamp()
    }

    fn balance(&self, who: &Address) -> i128 {
        self.token_client.balance(who)
    }

    fn pool(&self) -> i128 {
        self.token_client.balance(&self.contract_id)
    }

    fn get(&self, stream_id: u64) -> crate::Stream {
        self.client.get_stream(&stream_id)
    }

    fn ids(&self, ids: &[u64]) -> soroban_sdk::Vec<u64> {
        soroban_sdk::Vec::from_slice(&self.env, ids)
    }

    /// Simple linear stream with all capabilities, no cliff.
    fn create_simple(&self, deposit: i128, duration: u64) -> u64 {
        let start = self.now();
        self.client.create_stream(
            &self.sender,
            &self.recipient,
            &self.token,
            &deposit,
            &start,
            &(start + duration),
            &start,
            &true,
            &true,
            &true,
        )
    }

    /// Full-control stream creation.
    #[allow(clippy::too_many_arguments)]
    fn create(
        &self,
        deposit: i128,
        start: u64,
        end: u64,
        cliff: u64,
        cancellable: bool,
        pausable: bool,
        transferable: bool,
    ) -> u64 {
        self.client.create_stream(
            &self.sender,
            &self.recipient,
            &self.token,
            &deposit,
            &start,
            &end,
            &cliff,
            &cancellable,
            &pausable,
            &transferable,
        )
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Snapshot of every field that `apply_withdrawal` may touch, plus balances.
#[derive(Debug, PartialEq, Eq)]
struct Snapshot {
    withdrawn: i128,
    status: StreamStatus,
    paused_at: Option<u64>,
    paused_total: u64,
    recipient_balance: i128,
    pool_balance: i128,
}

impl Snapshot {
    fn take_revocable(h: &RevocableHarness, stream_id: u64) -> Self {
        let s = h.get(stream_id);
        Snapshot {
            withdrawn: s.withdrawn,
            status: s.status,
            paused_at: s.paused_at,
            paused_total: s.paused_total,
            recipient_balance: h.balance(&h.recipient),
            pool_balance: h.pool(),
        }
    }
}

// ---------------------------------------------------------------------------
// Tests — SAC authorization revoked
// ---------------------------------------------------------------------------

/// Deauthorizing the recipient causes the SAC to reject the token transfer.
/// The withdraw call must panic (host trap), and stream state and balances must
/// be unchanged afterwards.
///
/// After re-authorizing, a retry must succeed and pay exactly the same amount.
#[test]
fn withdraw_deauthorized_leaves_no_bookkeeping_side_effects() {
    let h = RevocableHarness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(30 * DAY);

    // Expected withdrawable balance before and after the failed attempt.
    let expected_withdrawable = h.client.withdrawable_of(&id);
    assert_eq!(expected_withdrawable, 300 * ONE);

    let before = Snapshot::take_revocable(&h, id);
    assert_eq!(before.withdrawn, 0);
    assert_eq!(before.status, StreamStatus::Active);
    assert_eq!(before.recipient_balance, 0);

    // Deauthorize recipient so the token transfer fails.
    h.token_admin.set_authorized(&h.recipient, &false);

    // Catch the panic — in the test host a trapping sub-call unwinds.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        h.client.withdraw(&id, &None);
    }));
    assert!(result.is_err(), "withdraw must panic when transfer is rejected");

    // State must be byte-for-byte identical to before the attempt.
    let after = Snapshot::take_revocable(&h, id);
    assert_eq!(
        before, after,
        "apply_withdrawal must not leave any permanent bookkeeping side effects \
         when the token transfer fails (SAC deauthorized recipient)",
    );

    // The withdrawable amount must also be unchanged: nothing was marked as paid out.
    assert_eq!(
        h.client.withdrawable_of(&id),
        expected_withdrawable,
        "withdrawable_of must be unchanged after a failed withdrawal",
    );

    // Re-authorize and verify a clean retry succeeds.
    h.token_admin.set_authorized(&h.recipient, &true);
    let paid = h.client.withdraw(&id, &None);
    assert_eq!(
        paid, expected_withdrawable,
        "retry after re-authorization must pay the full accrued amount",
    );
    assert_eq!(h.balance(&h.recipient), expected_withdrawable);

    // Pool must exactly equal outstanding liability.
    let stream_after = h.get(id);
    assert_eq!(
        h.pool(),
        stream_after.deposited - stream_after.withdrawn,
        "pool must equal outstanding liability after successful retry",
    );
}

/// Same guarantee for the status transition: a stream that would be marked
/// `Depleted` by the withdrawal is NOT permanently set to `Depleted` when the
/// transfer fails.
#[test]
fn status_is_not_permanently_depleted_when_transfer_fails() {
    let h = RevocableHarness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    // Advance past end so the full deposit is withdrawable.
    h.warp_to(T0 + 100 * DAY);

    assert_eq!(h.client.withdrawable_of(&id), 1_000 * ONE);
    let before = Snapshot::take_revocable(&h, id);
    assert_eq!(before.status, StreamStatus::Active);

    // Deauthorize so the transfer fails after state is tentatively written.
    h.token_admin.set_authorized(&h.recipient, &false);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        h.client.withdraw(&id, &None);
    }));
    assert!(result.is_err(), "withdraw must panic");

    let after = Snapshot::take_revocable(&h, id);
    assert_eq!(
        before, after,
        "stream must not be permanently marked Depleted when transfer fails",
    );
    assert_eq!(after.status, StreamStatus::Active, "status must remain Active");
    assert_eq!(
        after.withdrawn, 0,
        "withdrawn must still be zero after failed attempt",
    );
    assert_eq!(
        h.client.withdrawable_of(&id),
        1_000 * ONE,
        "withdrawable must be unchanged after failed depletion attempt",
    );

    // Re-authorize and drain cleanly.
    h.token_admin.set_authorized(&h.recipient, &true);
    h.client.withdraw(&id, &None);
    assert_eq!(h.get(id).status, StreamStatus::Depleted);
    let stream = h.get(id);
    assert_eq!(
        h.pool(),
        stream.deposited - stream.withdrawn,
        "pool must equal outstanding liability after depletion",
    );
}

/// A stream that would be marked `Depleted` when drained and had an in-progress
/// pause must not have its `paused_at` / `paused_total` tampered when the
/// transfer fails.
#[test]
fn paused_stream_bookkeeping_is_unchanged_when_transfer_fails() {
    let h = RevocableHarness::new();
    let id = h.create(1_000 * ONE, T0, T0 + 100 * DAY, T0, true, true, true);
    // Advance, then pause so the stream is both paused and has earned accrual.
    h.advance(40 * DAY);
    h.client.pause(&id);
    // The recipient can still withdraw earned tokens while paused.
    assert!(h.client.withdrawable_of(&id) > 0);

    let before = Snapshot::take_revocable(&h, id);
    assert_eq!(before.status, StreamStatus::Paused);

    h.token_admin.set_authorized(&h.recipient, &false);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        h.client.withdraw(&id, &None);
    }));
    assert!(result.is_err(), "withdraw must panic");

    let after = Snapshot::take_revocable(&h, id);
    assert_eq!(
        before, after,
        "paused_at, paused_total, and status must not change when transfer fails",
    );
    assert_eq!(after.paused_at, before.paused_at, "paused_at must be unchanged");
    assert_eq!(after.status, StreamStatus::Paused, "status must remain Paused");
    assert_eq!(
        after.withdrawn, before.withdrawn,
        "withdrawn must be unchanged",
    );
}

/// Explicit partial withdraw with deauthorized recipient: same atomicity
/// guarantee for the `Some(amount)` code path.
#[test]
fn partial_withdraw_deauthorized_leaves_no_bookkeeping_side_effects() {
    let h = RevocableHarness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(60 * DAY);

    // Request less than the full withdrawable amount (explicit partial).
    let partial = 100 * ONE;
    let before = Snapshot::take_revocable(&h, id);

    h.token_admin.set_authorized(&h.recipient, &false);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        h.client.withdraw(&id, &Some(partial));
    }));
    assert!(result.is_err(), "withdraw must panic");

    let after = Snapshot::take_revocable(&h, id);
    assert_eq!(
        before, after,
        "partial withdraw must not leave any permanent bookkeeping side effects \
         when the token transfer fails",
    );

    // Re-authorize: retry with the same partial amount must succeed.
    h.token_admin.set_authorized(&h.recipient, &true);
    let paid = h.client.withdraw(&id, &Some(partial));
    assert_eq!(paid, partial);
    assert_eq!(h.balance(&h.recipient), partial);
    let stream = h.get(id);
    assert_eq!(
        h.pool(),
        stream.deposited - stream.withdrawn,
        "pool must equal outstanding liability after partial withdraw",
    );
}

/// Same guarantee for `batch_withdraw`.
#[test]
fn batch_withdraw_deauthorized_leaves_no_bookkeeping_side_effects() {
    let h = RevocableHarness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(30 * DAY);

    let expected_withdrawable = h.client.withdrawable_of(&id);
    assert_eq!(expected_withdrawable, 300 * ONE);

    let before = Snapshot::take_revocable(&h, id);

    // Deauthorize recipient so the transfer inside batch_withdraw fails.
    h.token_admin.set_authorized(&h.recipient, &false);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        h.client.batch_withdraw(&h.recipient, &h.ids(&[id]));
    }));
    assert!(result.is_err(), "batch_withdraw must panic when transfer is rejected");

    let after = Snapshot::take_revocable(&h, id);
    assert_eq!(
        before, after,
        "batch_withdraw must not leave any permanent bookkeeping side effects \
         when the token transfer fails",
    );

    // Re-authorize and verify a clean retry succeeds.
    h.token_admin.set_authorized(&h.recipient, &true);
    let total = h.client.batch_withdraw(&h.recipient, &h.ids(&[id]));
    assert_eq!(total, expected_withdrawable);
    assert_eq!(h.balance(&h.recipient), expected_withdrawable);
    let stream = h.get(id);
    assert_eq!(
        h.pool(),
        stream.deposited - stream.withdrawn,
        "pool must equal outstanding liability after batch_withdraw retry",
    );
}

// ---------------------------------------------------------------------------
// Tests — always-panicking token contract
// ---------------------------------------------------------------------------

/// Register a contract stub that always panics in `transfer` at the existing
/// token address, proving that even a hard host-level trap leaves no permanent
/// bookkeeping side effects.
#[test]
fn withdraw_always_panicking_token_leaves_no_side_effects() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(50 * DAY);

    let expected_withdrawable = h.client.withdrawable_of(&id);
    assert_eq!(expected_withdrawable, 500 * ONE);

    // Snapshot what matters: only stream storage fields (no token balance
    // lookups after we swap the contract, since the stub doesn't implement balance).
    let stream_before = h.get(id);

    // Swap the real SAC for a contract that always panics on transfer.
    h.env.register_at(&h.token, AlwaysPanicsToken, ());

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        h.client.withdraw(&id, &None);
    }));
    assert!(result.is_err(), "withdraw must panic with always-panicking token");

    // Stream storage must be completely unchanged.
    let stream_after = h.get(id);
    assert_eq!(
        stream_after.withdrawn, stream_before.withdrawn,
        "withdrawn must not be permanently incremented after host trap",
    );
    assert_eq!(
        stream_after.status, stream_before.status,
        "status must not be permanently changed after host trap",
    );
    assert_eq!(
        stream_after.paused_at, stream_before.paused_at,
        "paused_at must not be changed after host trap",
    );
    assert_eq!(
        stream_after.paused_total, stream_before.paused_total,
        "paused_total must not be changed after host trap",
    );
    assert_eq!(
        stream_after.deposited, stream_before.deposited,
        "deposited must not be changed after host trap",
    );
}

/// Same assertion for `batch_withdraw` with the always-panicking token.
#[test]
fn batch_withdraw_always_panicking_token_leaves_no_side_effects() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(40 * DAY);

    let stream_before = h.get(id);
    let expected_withdrawable = h.client.withdrawable_of(&id);
    assert_eq!(expected_withdrawable, 400 * ONE);

    // Swap in the always-panicking token.
    h.env.register_at(&h.token, AlwaysPanicsToken, ());

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        h.client.batch_withdraw(&h.recipient, &h.ids(&[id]));
    }));
    assert!(result.is_err(), "batch_withdraw must panic with always-panicking token");

    let stream_after = h.get(id);
    assert_eq!(
        stream_after.withdrawn, stream_before.withdrawn,
        "batch_withdraw: withdrawn must not be permanently incremented after host trap",
    );
    assert_eq!(
        stream_after.status, stream_before.status,
        "batch_withdraw: status must not be permanently changed after host trap",
    );
}
