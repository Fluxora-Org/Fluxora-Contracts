//! Shared test harness.

#![allow(dead_code)]

use soroban_sdk::testutils::storage::Persistent as _;
use soroban_sdk::testutils::{Address as _, Ledger as _};
use soroban_sdk::token::{Client as TokenClient, StellarAssetClient};
use soroban_sdk::{Address, Env, Vec};

use crate::{
    accrual, storage, CliffMode, DataKey, FluxoraStream, FluxoraStreamClient, Stream, StreamStatus,
};

// ---------------------------------------------------------------------------
// TestSnapshot — deterministic, credential-free state capture
// ---------------------------------------------------------------------------

/// A point-in-time snapshot of ledger, stream, and token state.
///
/// Captured by [`Harness::snapshot`] and printed automatically on assertion
/// failure via `eprintln!`. Every value is a plain integer or enum — no
/// addresses, no secrets. The output is deterministic and replayable: given the
/// same seed inputs, the same snapshot is reproduced every time.
///
/// # CI safety
///
/// Soroban test environments use randomly-generated, ephemeral addresses that
/// exist only in the in-memory test host. There are no private keys, RPC
/// endpoints, or credentials of any kind. Printing a snapshot in CI output is
/// therefore safe and produces no information useful to an attacker.
///
/// # What it captures
///
/// | Field | Source |
/// |---|---|
/// | `ledger_timestamp` | `env.ledger().timestamp()` |
/// | `ledger_sequence` | `env.ledger().sequence()` |
/// | `stream_count` | `client.stream_count()` |
/// | `streams` | one [`StreamSnapshot`] per stream id |
/// | `balance_sender` | `token_client.balance(sender)` |
/// | `balance_recipient` | `token_client.balance(recipient)` |
/// | `balance_other` | `token_client.balance(other)` |
/// | `balance_pool` | `token_client.balance(contract_id)` |
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestSnapshot {
    /// Unix seconds at the moment the snapshot was taken.
    pub ledger_timestamp: u64,
    /// Ledger sequence number at the moment the snapshot was taken.
    pub ledger_sequence: u32,
    /// Total number of streams in the contract.
    pub stream_count: u64,
    /// Per-stream accounting state, indexed by stream id.
    pub streams: std::vec::Vec<StreamSnapshot>,
    /// Token balance of the sender address, in stroops.
    pub balance_sender: i128,
    /// Token balance of the recipient address, in stroops.
    pub balance_recipient: i128,
    /// Token balance of the `other` address, in stroops.
    pub balance_other: i128,
    /// Token balance pooled inside the contract, in stroops.
    pub balance_pool: i128,
}

/// Accounting snapshot for a single stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamSnapshot {
    /// Stream identifier (monotonic, starts at 0).
    pub id: u64,
    /// Total ever deposited (reduced on cancel).
    pub deposited: i128,
    /// Total ever withdrawn by the recipient.
    pub withdrawn: i128,
    /// Computed vested amount at the snapshot instant.
    pub vested: i128,
    /// Computed withdrawable amount at the snapshot instant.
    pub withdrawable: i128,
    /// Current lifecycle status.
    pub status: StreamStatus,
    /// Schedule start (unix seconds).
    pub start_time: u64,
    /// Schedule end (unix seconds).
    pub end_time: u64,
    /// Cliff gate (unix seconds).
    pub cliff_time: u64,
    /// Which clock the cliff gate is read against.
    pub cliff_mode: CliffMode,
    /// Cumulative seconds spent paused (excluding any in-progress pause).
    pub paused_total: u64,
    /// Freeze point if the stream is currently paused.
    pub paused_at: Option<u64>,
}

impl std::fmt::Display for StreamSnapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "stream[{id}]: status={status:?} \
             deposited={dep} withdrawn={wth} vested={vest} withdrawable={draw} \
             start={start} end={end} cliff={cliff} cliff_mode={cmode:?} \
             paused_total={ptot}{paused_at}",
            id = self.id,
            status = self.status,
            dep = self.deposited,
            wth = self.withdrawn,
            vest = self.vested,
            draw = self.withdrawable,
            start = self.start_time,
            end = self.end_time,
            cliff = self.cliff_time,
            cmode = self.cliff_mode,
            ptot = self.paused_total,
            paused_at = match self.paused_at {
                Some(t) => std::format!(" paused_at={t}"),
                None => std::string::String::new(),
            },
        )
    }
}

impl std::fmt::Display for TestSnapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::writeln!(f, "=== TestSnapshot ===")?;
        std::writeln!(
            f,
            "ledger: timestamp={ts} sequence={seq}",
            ts = self.ledger_timestamp,
            seq = self.ledger_sequence,
        )?;
        std::writeln!(
            f,
            "balances: sender={s} recipient={r} other={o} pool={p}",
            s = self.balance_sender,
            r = self.balance_recipient,
            o = self.balance_other,
            p = self.balance_pool,
        )?;
        std::writeln!(f, "streams ({}):", self.stream_count)?;
        for ss in &self.streams {
            std::writeln!(f, "  {ss}")?;
        }
        write!(f, "===================")?;
        Ok(())
    }
}

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
    /// Also advances the sequence number at the network's *nominal* close
    /// cadence, so that time-based tests exercise TTL decay realistically
    /// rather than freezing the sequence while the clock runs. The nominal
    /// rate is deliberate: this simulates the network the contract runs on,
    /// while [`storage::seconds_to_ledgers`] carries the funding margin on
    /// the contract side. Deriving cadence from the margined conversion would
    /// hide the very gap the margin exists to cover.
    pub fn advance(&self, seconds: u64) {
        let info = self.env.ledger().get();
        self.env.ledger().set_timestamp(info.timestamp + seconds);
        let ledgers = seconds
            .saturating_add(storage::SECONDS_PER_LEDGER - 1)
            .saturating_div(storage::SECONDS_PER_LEDGER);
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

    /// Remaining TTL, in ledgers, of a stream entry.
    pub fn ttl_of(&self, stream_id: u64) -> u32 {
        self.env.as_contract(&self.contract_id, || {
            self.env
                .storage()
                .persistent()
                .get_ttl(&DataKey::Stream(stream_id))
        })
    }

    /// The largest TTL any entry can actually hold right now.
    ///
    /// This is deliberately read from the SDK rather than from
    /// `LedgerInfo::max_entry_ttl`: the achievable maximum is
    /// `max_live_until_ledger - sequence`, which is not always the raw
    /// configured value. Asserting against the config number bakes in an
    /// off-by-one.
    pub fn max_achievable_ttl(&self) -> u32 {
        self.env
            .as_contract(&self.contract_id, || self.env.storage().max_ttl())
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
            &None,
        )
    }

    /// A pending stream over `duration`, no cliff, all capabilities on.
    ///
    /// Uses `create_stream_pending`, so the stream starts in `Pending` and
    /// accrues nothing until `accept_stream` is called.
    pub fn create_pending(&self, deposit: i128, duration: u64) -> u64 {
        let start = self.now();
        self.client.create_stream_pending(
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

    /// Full control over every parameter of a pending stream.
    #[allow(clippy::too_many_arguments)]
    pub fn create_pending_full(
        &self,
        deposit: i128,
        start: u64,
        end: u64,
        cliff: u64,
        cancellable: bool,
        pausable: bool,
        transferable: bool,
    ) -> u64 {
        self.client.create_stream_pending(
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
            &None,
        )
    }

    /// Create with reference support - simple case
    pub fn create_simple_with_ref(
        &self,
        deposit: i128,
        duration: u64,
        reference: Option<soroban_sdk::String>,
    ) -> u64 {
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
            &reference,
        )
    }

    /// Create with reference support - full control
    #[allow(clippy::too_many_arguments)]
    pub fn create_with_ref(
        &self,
        deposit: i128,
        start: u64,
        end: u64,
        cliff: u64,
        cancellable: bool,
        pausable: bool,
        transferable: bool,
        reference: Option<soroban_sdk::String>,
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
            &reference,
        )
    }

    /// Full control over every creation parameter, including which clock the
    /// cliff gate is read against.
    #[allow(clippy::too_many_arguments)]
    pub fn create_with_cliff_mode(
        &self,
        deposit: i128,
        start: u64,
        end: u64,
        cliff: u64,
        cliff_mode: CliffMode,
        cancellable: bool,
        pausable: bool,
        transferable: bool,
    ) -> u64 {
        self.client.create_stream_with_cliff_mode(
            &self.sender,
            &self.recipient,
            &self.token,
            &deposit,
            &start,
            &end,
            &cliff,
            &cliff_mode,
            &cancellable,
            &pausable,
            &transferable,
            &None,
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
        self.assert_pool_invariant_for(&self.token);
    }

    /// [`assert_pool_invariant`](Self::assert_pool_invariant) against a token
    /// other than the harness's own.
    ///
    /// Tests that need a second asset — a dedicated supply, a different
    /// issuer, a hostile token — still must assert the pool invariant on it,
    /// but the harness-shaped check above is hard-wired to [`Harness::token`].
    /// This is the same check with the token made explicit.
    pub fn assert_pool_invariant_for(&self, token: &Address) {
        self.assert_invariants();
        let total = self.outstanding_liability(token);
        let pool = TokenClient::new(&self.env, token).balance(&self.contract_id);
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
        self.assert_pool_exact_for(&self.token);
    }

    /// [`assert_pool_exact`](Self::assert_pool_exact) against a token other
    /// than the harness's own. See
    /// [`assert_pool_invariant_for`](Self::assert_pool_invariant_for).
    pub fn assert_pool_exact_for(&self, token: &Address) {
        self.assert_invariants();
        let total = self.outstanding_liability(token);
        let pool = TokenClient::new(&self.env, token).balance(&self.contract_id);
        assert_eq!(
            pool, total,
            "pooled balance and outstanding liability diverged",
        );
    }

    /// Sum of `deposited - withdrawn` across every stream denominated in
    /// `token` (see [`accrual::liability`]).
    fn outstanding_liability(&self, token: &Address) -> i128 {
        let mut total: i128 = 0;
        let count = self.client.stream_count();
        for id in 0..count {
            let stream = self.client.get_stream(&id);
            if stream.token != *token {
                continue;
            }
            total += accrual::liability(&stream).expect("liability must not overflow");
        }
        total
    }

    // -----------------------------------------------------------------------
    // Counter / population consistency (#1699)
    // -----------------------------------------------------------------------

    /// See [`assert_stream_count_consistent`]. The harness-shaped wrapper, so
    /// the check reads like its siblings (`assert_pool_exact`, …).
    pub fn assert_stream_count_consistent(&self) {
        assert_stream_count_consistent(&self.env, &self.contract_id);
    }

    // -----------------------------------------------------------------------
    // Snapshot helpers
    // -----------------------------------------------------------------------

    /// Capture a deterministic, credential-free snapshot of the current ledger
    /// and token state.
    ///
    /// The snapshot encodes everything needed to reproduce and diagnose a test
    /// failure:
    ///
    /// * **Ledger state** — timestamp and sequence number are the two inputs
    ///   that govern all accrual calculations; given these a developer can
    ///   reconstruct every vested/withdrawable value by hand.
    /// * **Stream accounting** — `deposited`, `withdrawn`, `vested`,
    ///   `withdrawable`, status, and the full schedule for every stream.
    /// * **Token balances** — sender, recipient, other, and the contract pool.
    ///   Together with the stream accounting these let you verify the pool
    ///   invariant without re-running the suite.
    ///
    /// # Usage in tests
    ///
    /// Take a snapshot before the operation under test; if the assertion fails,
    /// print it so CI output contains the full pre-condition state:
    ///
    /// ```rust,ignore
    /// let snap = h.snapshot();
    /// let result = h.client.try_withdraw(&id, &None);
    /// assert!(result.is_ok(), "withdraw failed\n{snap}");
    /// ```
    ///
    /// For `should_panic` tests or multi-step sequences, calling
    /// [`Harness::dump_snapshot`] inside the test body lets you print without
    /// storing the value.
    pub fn snapshot(&self) -> TestSnapshot {
        let info = self.env.ledger().get();
        let now = info.timestamp;
        let count = self.client.stream_count();

        let streams = (0..count)
            .map(|id| {
                let s: Stream = self.client.get_stream(&id);
                let vest = accrual::vested(&s, now).expect("vested must not overflow in snapshot");
                let draw = accrual::withdrawable(&s, now).expect("withdrawable must not overflow");
                StreamSnapshot {
                    id,
                    deposited: s.deposited,
                    withdrawn: s.withdrawn,
                    vested: vest,
                    withdrawable: draw,
                    status: s.status,
                    start_time: s.start_time,
                    end_time: s.end_time,
                    cliff_time: s.cliff_time,
                    cliff_mode: s.cliff_mode,
                    paused_total: s.paused_total,
                    paused_at: s.paused_at,
                }
            })
            .collect();

        TestSnapshot {
            ledger_timestamp: now,
            ledger_sequence: info.sequence_number,
            stream_count: count,
            streams,
            balance_sender: self.balance(&self.sender),
            balance_recipient: self.balance(&self.recipient),
            balance_other: self.balance(&self.other),
            balance_pool: self.pool(),
        }
    }

    /// Print the current snapshot to stderr.
    ///
    /// Useful inside test bodies where you want the state visible in
    /// `--nocapture` output without having to store the snapshot in a variable.
    /// Nothing is written when the test passes; you only see it when you run
    /// with `-- --nocapture` or when the binary dumps stderr on a panic.
    pub fn dump_snapshot(&self) {
        std::eprintln!("{}", self.snapshot());
    }
}

// ---------------------------------------------------------------------------
// Issue #1699 — counter / population consistency
// ---------------------------------------------------------------------------

/// **`stream_count()` must equal the number of streams that actually exist.**
///
/// The contract keeps two representations of the same fact: the instance-level
/// id counter behind [`crate::FluxoraStream::stream_count`] and the population
/// of `DataKey::Stream(id)` records in persistent storage. This asserts they
/// still agree, which is what the following invariants jointly guarantee:
///
/// * ids are handed out from the counter, contiguously, and never reused, so a
///   successful create adds exactly one record at exactly the next id;
/// * no entry point ever *removes* a stream record — a terminal operation
///   rewrites it in place — so the population only grows;
/// * a failing invocation rolls back **every** write it made, counter bumps
///   included, so a create (or a terminal operation) that fails partway
///   cannot leave the two halves of a write behind.
///
/// # How the probe works
///
/// Ids are contiguous, so the population is exactly `0..stream_count()`. The
/// check counts the records that exist over the **inclusive** range
/// `0..=stream_count()` — inclusive so that the *next* id out of the counter is
/// probed too — and requires that count to come back as `stream_count()`:
///
/// * counter **too high**: some id below the counter has no record, so the
///   population counts short and the assertion fails;
/// * counter **too low**: the record sitting at (and above) the counter still
///   exists, so the population counts long and the assertion fails.
///
/// One count therefore detects a wrong counter in either direction, and a
/// deleted record (a hole) as well.
///
/// # When the two legitimately disagree — do not call it here
///
/// On a real network an entry whose TTL has run out archives and reports
/// `stream_exists() == false` while its id is still below the counter: that is
/// the documented "needs restoring" state, not a divergence
/// (`docs/KNOWN-LIMITATIONS.md`). The SDK test host auto-restores on read, so
/// time alone cannot produce this state in tests — only an explicit
/// `persistent().remove(...)` can. Likewise, the `test::create` exhaustion
/// fixtures seed the counter at `u64::MAX`, which would make the probe below
/// iterate for ~2^64 ids; do not call this while the counter is seeded high.
pub fn assert_stream_count_consistent(env: &Env, contract_id: &Address) {
    let client = FluxoraStreamClient::new(env, contract_id);
    let count = client.stream_count();

    let existing = (0..=count).filter(|&id| client.stream_exists(&id)).count() as u64;

    assert_eq!(
        existing, count,
        "stream_count() reports {count} streams but only {existing} stream \
         records exist (ids 0..={count} probed) — counter and population diverged",
    );
}
