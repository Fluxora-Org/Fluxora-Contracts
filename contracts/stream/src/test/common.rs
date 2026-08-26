//! Shared test harness.

#![allow(dead_code)]

use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::token::{Client as TokenClient, StellarAssetClient};
use soroban_sdk::{Address, Env, Vec};

use crate::{accrual, storage, FluxoraStream, FluxoraStreamClient, Stream, StreamStatus};

/// USDC on Stellar has 7 decimals. Not 6, not 18.
pub const DECIMALS: u32 = 7;
/// One whole token unit, in stroops.
pub const ONE: i128 = 10_000_000;

pub const DAY: u64 = 86_400;
pub const YEAR: u64 = 365 * DAY;

/// Arbitrary non-zero epoch so tests never accidentally depend on `now == 0`,
/// which would mask sign and underflow bugs.
pub const T0: u64 = 1_700_000_000;

pub struct Harness<'a> {
    pub env: Env,
    pub client: FluxoraStreamClient<'a>,
    pub contract_id: Address,
    pub token: Address,
    pub token_client: TokenClient<'a>,
    pub token_admin: StellarAssetClient<'a>,
    pub sender: Address,
    pub recipient: Address,
    pub other: Address,
}

impl<'a> Harness<'a> {
    /// Fresh environment with all auth mocked, one SAC token, and a funded
    /// sender. Ledger time starts at [`T0`].
    pub fn new() -> Harness<'a> {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(T0);

        let contract_id = env.register(FluxoraStream, ());
        let client = FluxoraStreamClient::new(&env, &contract_id);

        let issuer = Address::generate(&env);
        let asset = env.register_stellar_asset_contract_v2(issuer);
        let token = asset.address();

        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);
        let other = Address::generate(&env);

        let token_admin = StellarAssetClient::new(&env, &token);
        token_admin.mint(&sender, &(1_000_000 * ONE));
        token_admin.mint(&other, &(1_000_000 * ONE));

        let token_client = TokenClient::new(&env, &token);
        Harness {
            client,
            contract_id,
            token: token.clone(),
            token_client,
            token_admin,
            sender,
            recipient,
            other,
            env,
        }
    }

    /// Advance the ledger clock by `seconds`.
    ///
    /// Also advances the sequence number at the nominal ledger close rate, so
    /// that time-based tests exercise TTL decay realistically rather than
    /// freezing the sequence while the clock runs.
    pub fn advance(&self, seconds: u64) {
        let info = self.env.ledger().get();
        self.env.ledger().set_timestamp(info.timestamp + seconds);
        let ledgers = storage::seconds_to_ledgers(seconds);
        self.env
            .ledger()
            .set_sequence_number(info.sequence_number.saturating_add(ledgers));
    }

    /// Jump to an absolute timestamp.
    pub fn warp_to(&self, timestamp: u64) {
        let info = self.env.ledger().get();
        if timestamp > info.timestamp {
            self.advance(timestamp - info.timestamp);
        } else {
            self.env.ledger().set_timestamp(timestamp);
        }
    }

    pub fn now(&self) -> u64 {
        self.env.ledger().timestamp()
    }

    pub fn balance(&self, who: &Address) -> i128 {
        self.token_client.balance(who)
    }

    /// Tokens currently pooled in the contract.
    pub fn pool(&self) -> i128 {
        self.token_client.balance(&self.contract_id)
    }

    /// A plain linear stream over `duration`, no cliff, all capabilities on.
    pub fn create_simple(&self, deposit: i128, duration: u64) -> u64 {
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

    /// Full control over every creation parameter.
    #[allow(clippy::too_many_arguments)]
    pub fn create(
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

    pub fn get(&self, stream_id: u64) -> Stream {
        self.client.get_stream(&stream_id)
    }

    pub fn ids(&self, ids: &[u64]) -> Vec<u64> {
        Vec::from_slice(&self.env, ids)
    }

    /// **Post-condition bundle: the stated invariants I1, I4 and I5.**
    ///
    /// Asserted after every operation in the suite. See the `accrual` module
    /// docs for the full statement.
    ///
    /// I3 (`vested` never moves backwards across a call) cannot be checked from
    /// a single snapshot — it needs a before/after pair at a fixed instant — so
    /// it lives in [`assert_no_vested_regression`](Self::assert_no_vested_regression)
    /// and is exercised exhaustively by `test::monotonicity`.
    pub fn assert_invariants(&self) {
        let now = self.now();
        for id in 0..self.client.stream_count() {
            let s = self.client.get_stream(&id);
            let vested = accrual::vested(&s, now).expect("vested must not overflow");

            // I1 — bounds.
            assert!(s.withdrawn >= 0, "stream {id}: negative withdrawn");
            assert!(s.deposited >= 0, "stream {id}: negative deposit");
            assert!(
                s.withdrawn <= vested,
                "stream {id}: I1 violated — withdrawn {} exceeds vested {vested}",
                s.withdrawn,
            );
            assert!(
                vested <= s.deposited,
                "stream {id}: I1 violated — vested {vested} exceeds deposited {}",
                s.deposited,
            );

            // I4 — conservation, exactly.
            let refundable = accrual::refundable(&s, now).expect("refundable must not overflow");
            assert_eq!(
                vested + refundable,
                s.deposited,
                "stream {id}: I4 violated — conservation broken",
            );

            // I5 — pause coherence.
            match s.status {
                StreamStatus::Paused => assert!(
                    s.paused_at.is_some(),
                    "stream {id}: I5 violated — Paused with no freeze point",
                ),
                other => assert!(
                    s.paused_at.is_none(),
                    "stream {id}: I5 violated — {other:?} but still frozen",
                ),
            }

            // Schedule sanity: a cancel must never invert the range.
            assert!(s.end_time >= s.start_time, "stream {id}: inverted schedule",);
        }
    }

    /// Snapshot `vested` for every stream at the current instant.
    ///
    /// Pair with [`assert_no_vested_regression`](Self::assert_no_vested_regression)
    /// around a call to check invariant I3.
    pub fn vested_snapshot(&self) -> std::vec::Vec<i128> {
        let now = self.now();
        (0..self.client.stream_count())
            .map(|id| {
                accrual::vested(&self.client.get_stream(&id), now)
                    .expect("vested must not overflow")
            })
            .collect()
    }

    /// **Invariant I3.** No operation may reduce `vested(t)` for a fixed `t`.
    ///
    /// The clock must not have advanced between the snapshot and this call, or
    /// the comparison is meaningless — that is why `test::monotonicity` freezes
    /// time around each operation.
    pub fn assert_no_vested_regression(&self, before: &[i128], label: &str) {
        let after = self.vested_snapshot();
        for (id, prev) in before.iter().enumerate() {
            let now = after[id];
            assert!(
                now >= *prev,
                "{label}: I3 violated — stream {id} vested moved backwards, \
                 {prev} -> {now} at a fixed instant",
            );
        }
    }

    /// **The pool invariant.**
    ///
    /// The contract's pooled token balance must always be at least the sum of
    /// every stream's outstanding liability (`deposited - withdrawn`). If this
    /// ever fails, some stream's claim is unbacked and a recipient somewhere
    /// cannot be paid.
    ///
    /// Call this after every operation. It is the single most important
    /// assertion in the suite.
    pub fn assert_pool_invariant(&self) {
        self.assert_invariants();
        let mut total: i128 = 0;
        let count = self.client.stream_count();
        for id in 0..count {
            let stream = self.client.get_stream(&id);
            if stream.token != self.token {
                continue;
            }
            total += accrual::liability(&stream).expect("liability must not overflow");
        }
        let pool = self.pool();
        assert!(
            pool >= total,
            "pool invariant violated: pooled balance {pool} < outstanding liability {total}",
        );
    }

    /// The pool must hold *exactly* the outstanding liability: any excess means
    /// tokens are stranded in the contract with no stream accounting for them.
    ///
    /// Stronger than [`assert_pool_invariant`](Self::assert_pool_invariant) and
    /// true for every test that does not deliberately donate loose tokens to the
    /// contract.
    pub fn assert_pool_exact(&self) {
        self.assert_invariants();
        let mut total: i128 = 0;
        let count = self.client.stream_count();
        for id in 0..count {
            let stream = self.client.get_stream(&id);
            if stream.token != self.token {
                continue;
            }
            total += accrual::liability(&stream).expect("liability must not overflow");
        }
        assert_eq!(
            self.pool(),
            total,
            "pooled balance and outstanding liability diverged",
        );
    }
}
