#![no_std]
//! # Fluxora — continuous payment streaming for Soroban
//!
//! Lock tokens once; have them accrue continuously to a recipient over time.
//! The recipient pulls their accrued balance whenever they like.
//!
//! This contract is a *primitive*, not an application. Payroll tools, grant
//! programs, subscription billing and vesting schedules are meant to be built
//! on top of it. Every scoping decision below favours generality on chain and
//! pushes convenience to the SDK.
//!
//! ## Pull-based by necessity
//!
//! Stellar has no scheduler — no cron, no keeper network, no way for a contract
//! to wake itself up. Every state change must be triggered by an external
//! transaction. So nothing here runs in the background: the recipient calls
//! [`FluxoraStream::withdraw`] and the contract computes what they have earned
//! at that instant.
//!
//! ## No on-chain stream discovery
//!
//! There is deliberately no per-user list of stream ids in storage. A `Vec<u64>`
//! of a treasury's streams grows without bound, costs rent forever, and blows
//! Soroban's per-transaction footprint limit once that treasury has a few
//! hundred recipients. On chain, a stream is only ever addressed by its `u64`
//! id. `test::resource_limits` states the payoff as a test: the 153rd stream
//! costs exactly what the 2nd did.
//!
//! Discovery is an off-chain concern: [`create_stream`](FluxoraStream::create_stream)
//! returns the new id and emits an event carrying sender, recipient and every
//! schedule field — including the [`ReleaseCurve`] — so an indexer can answer
//! "show me my streams" without the contract paying rent to remember.
//!
//! ## Immutable guarantees
//!
//! `cancellable`, `pausable` and `transferable` are fixed at creation and can
//! never change afterwards. This is a trust feature: before accepting a stream a
//! recipient can verify that the sender cannot claw it back, freeze it, or
//! reassign it. A stream that could *become* cancellable later would be
//! worthless as a guarantee.
//!
//! For the same reason the contract has no upgrade path, no fee switch, no
//! admin rotation and nothing user-configurable in storage. Immutability is
//! what lets another protocol depend on this one.
//!
//! **Upgrade posture: the contract is immutable.** There is no `upgrade`
//! entry point, no admin, and no storage slot that could authorise replacing
//! the deployed WASM. See `docs/ABI.md` "Upgrade posture" and
//! `docs/MIGRATION.md` for the consequences.
//!
//! ## The single operator: an opt-in emergency halt (#1818)
//!
//! The one exception is the contract-level emergency stop. An operator can be
//! installed exactly once, by [`set_halt_operator`](FluxoraStream::set_halt_operator);
//! that operator may then stop *settlement* across every stream
//! ([`halt`](FluxoraStream::halt)) and start it again
//! ([`resume_contract`](FluxoraStream::resume_contract)). The design keeps the
//! guarantees above intact:
//!
//! * **Opt-in.** A deployment that never installs an operator has none, cannot
//!   be halted, and behaves exactly as it did before the halt existed. The
//!   operator is unset by default, which is why no existing behaviour changed.
//! * **One-shot.** There is no rotation entry point. A second
//!   `set_halt_operator` is rejected, so the operator cannot be swapped mid
//!   incident — replacing one means deploying a new contract.
//! * **No reach into funds or schedules.** The operator cannot withdraw,
//!   cancel, pause, top up, transfer, or change any stream. It can only refuse
//!   new mutations contract-wide.
//! * **Reads keep answering.** Every view stays live while halted, so an
//!   integrator can still see balances and streams during an incident.
//! * **Explicit resume.** There is no timeout: settlement restarts when the
//!   operator says so, and both transitions emit an event.
//!
//! `test::halt` is the acceptance test: it halts the contract and drives every
//! mutating entry point to [`Error::ContractHalted`] while asserting each read
//! still answers.

#[cfg(all(target_family = "wasm", not(target_os = "none")))]
compile_error!(
    "Fluxora production WASM must be built for wasm32v1-none; wasm32-unknown-unknown can emit unsupported features."
);
#[cfg(all(target_family = "wasm", debug_assertions))]
compile_error!("Fluxora production WASM must be built without debug assertions; use --release.");
#[cfg(all(target_family = "wasm", feature = "testutils"))]
compile_error!("Fluxora production WASM must not enable the testutils feature.");

// The test suite runs against the host with `std` available; the contract
// itself is strictly `no_std`.
#[cfg(test)]
extern crate std;

mod accrual;
mod error;
mod events;
mod storage;
mod types;
#[cfg(test)]
mod protocol_limits;

pub use accrual::{
    cliff_reached, duration, elapsed, liability, refundable, stream_time, vested, withdrawable,
};
pub use error::Error;
pub use storage::{
    MIN_STREAM_TTL_LEDGERS, SECONDS_PER_LEDGER, TTL_BUFFER_SECONDS, TTL_SAFETY_MARGIN_PERCENT,
};
pub use types::op;
pub use types::{
    BatchCancelOutcome, BatchCreateRequest, CliffMode, DataKey, DelegateGrant, ReleaseCurve,
    Stream, StreamStatus, MAX_REFERENCE_LENGTH, MAX_STREAM_DURATION,
};

use soroban_sdk::{
    contract, contractimpl, contracttype, token, Address, Env, InvokeError, MuxedAddress, String,
    TryFromVal, Vec,
};

/// Mirror of `fluxora_factory::FactoryConfig`, used to decode the return value
/// of a cross-contract call to `FluxoraFactory::get_factory_config`.
///
/// `#[contracttype]` encodes structs as XDR maps keyed by field name, so this
/// decodes correctly as long as the field names and types match the factory's
/// definition — which they do, and `test::factory_policy_enforcement` verifies
/// the round-trip end-to-end.
///
/// This type is intentionally private to this module: it is only ever produced
/// by deserialising a factory response, never constructed directly.
#[contracttype]
#[derive(Clone)]
struct FactoryConfig {
    pub admin: Address,
    pub stream_contract: Address,
    pub max_deposit: i128,
    pub min_duration: u64,
    pub batch_cap_enforced: bool,
    pub creation_paused: bool,
    pub min_rate_per_second: Option<i128>,
    pub max_rate_per_second: Option<i128>,
}

/// Maximum number of streams one batch call may touch.
///
/// # Where this number comes from
///
/// Measured, not guessed. `test::resource_limits` reports the real cost of a
/// full batch against protocol 27's mainnet limits, and the constraint that
/// binds is not the one you would expect:
///
/// Measured evidence from the resource suite at the cap (16-stream batch):
///
/// | limit | used by a 16-stream batch | ceiling |
/// |---|---|---|
/// | total footprint (entries) | 43 | 400 |
/// | write entries | 20 | 200 |
/// | instructions | ~4.9M | 400M |
/// | **contract event bytes** | **9,984** | **16,384** |
///
/// Entry counts would allow well over a hundred streams per call. The *event
/// budget* allows about 26, because each stream emits a `withdrawn` event plus
/// the token contract's own `transfer` event — 624 bytes per stream between
/// them, and issue #1868's `sender` and pause bookkeeping on `withdrawn` are
/// what took that from 512.
///
/// Sixteen is that measured ceiling leaving 6,400 bytes of the event budget
/// spare. The margin is not decoration: the per-stream event cost depends on the
/// *token's* event payload, and a token heavier than the Stellar Asset Contract
/// used in the tests would inflate it. A cap that merely fits today would fail
/// on somebody else's token. The margin was the full 2x (8,192 bytes) before
/// #1868, and the cap is frozen ABI, so the payload cost is what the margin
/// absorbed rather than the cap moving.
///
/// Larger requests are rejected with [`Error::BatchTooLarge`] rather than
/// failing opaquely at the network level. The SDK chunks client-side, so the
/// exact value is invisible to integrators.
pub const MAX_BATCH_SIZE: u32 = 16;

/// Version of the public stream ABI inventory.
///
/// Bump this when making a *breaking* change to a public method: removing,
/// renaming, or changing a parameter/return type. Additive changes — a new
/// method, a new error discriminant, a field appended to an event payload —
/// do not require a bump; update `contracts/stream/abi/fluxora_stream.json`
/// so the snapshot stays the generated source of truth.
///
/// The on-chain contract is immutable, so a bump is a *new deployment*, not an
/// in-place upgrade. See `docs/ABI.md` and `test::abi`.
///
/// Immutability is deliberate: the contract exposes no upgrade entry point,
/// so a version bump can only ever ship as a fresh deployment with a new
/// contract id. See `docs/MIGRATION.md` for the migration path that follows
/// from that.
///
/// **2** — [`ReleaseCurve`] support. `get_stream`'s `Stream` return type grew a
/// `curve` field and `StreamCreated` grew a `curve` payload, so a typed client
/// built against version 1 cannot decode either. The new
/// [`FluxoraStream::create_stream_with_curve`] entry point is additive, and
/// [`FluxoraStream::create_stream`] keeps its exact signature and its linear
/// arithmetic; the stored layout is unchanged (see [`crate::types::StreamRecord`]).
/// # v2 — wall-clock cliff
///
/// Bumped for the one breaking change in this release: the `Stream` UDT gained
/// a `cliff_mode` field, which `test::abi` classifies as `type-changed UDT` and
/// therefore refuses without a version bump. `StreamCreated` also gained a
/// trailing `cliff_mode` payload field, and `create_stream_with_cliff_mode` a
/// new method — both additive on their own.
///
/// `create_stream`'s signature is deliberately **unchanged**, so v1 callers need
/// no migration: it now delegates with [`CliffMode::DEFAULT`]. `Stream` gaining
/// a field is only a break for a caller that *constructs* a `Stream` locally;
/// readers and indexers decode it from storage or an event, both of which carry
/// the new field explicitly.
pub const ABI_VERSION: u32 = 2;

/// Whether the deployed contract can be replaced in place.
///
/// **`false`, deliberately.** This constant exists so the posture is stated
/// in the ABI itself rather than only in prose: an integrator can read it
/// on-chain, and `test::abi` asserts it matches the absence of any upgrade
/// entry point.
///
/// # Consequences
///
/// * **No upgrade entry point.** The contract exposes no `upgrade`,
///   `set_admin`, or `migrate` method, and no storage key holds a WASM hash
///   or an admin address. `test::abi` fails if one is ever added without
///   flipping this constant and updating `docs/ABI.md`.
/// * **A new ABI version is a new deployment.** [`ABI_VERSION`] bumps ship
///   as a fresh contract id; existing streams stay on the old contract and
///   must be drained or cancelled there. `docs/MIGRATION.md` describes that
///   path.
/// * **The halt operator is not an upgrade path.** It can stop settlement
///   ([`FluxoraStream::halt`]) but cannot change code, storage layout, or
///   any stream's terms.
pub const UPGRADEABLE: bool = false;

/// Call `token.transfer(from, to, amount)` and map any failure to a stable
/// stream-level error.
///
/// # Why not forward the token's error discriminant?
///
/// A client receiving `Error(Contract, #N)` has no way to know whether `N`
/// comes from the stream contract or from the token contract without out-of-band
/// knowledge of which contract threw. Forwarding the raw token discriminant
/// would cause silent misinterpretation — e.g. token error #7 would decode as
/// `Unauthorized` against Fluxora's table, which is wrong and unsettling.
///
/// Instead, failures are bucketed into two stream-level categories that are
/// stable, unambiguous, and actionable:
///
/// * [`Error::TokenTransferFailed`] — the token returned a typed contract
///   error. The root cause (insufficient balance, authorization refused by the
///   token contract, etc.) is visible in the transaction's `diagnosticEvents`
///   and is therefore preserved for off-chain tooling without polluting the
///   stream ABI.
/// * [`Error::TokenMissing`] — the host raised an `Abort` (non-contract trap).
///   This most commonly means the `token` address has no deployed code. No
///   funds moved, so the stream is in a clean pre-transfer state.
fn token_transfer(
    env: &Env,
    token: &Address,
    from: &Address,
    to: MuxedAddress,
    amount: &i128,
) -> Result<(), Error> {
    match token::TokenClient::new(env, token).try_transfer(from, &to, amount) {
        Ok(Ok(())) => Ok(()),
        // The token contract returned a typed contract error (e.g. insufficient
        // balance, deauthorized trustline, custom token logic). The raw
        // discriminant is intentionally discarded — see the function doc.
        Err(Err(InvokeError::Contract(_))) | Ok(Err(_)) | Err(Ok(_)) => {
            Err(Error::TokenTransferFailed)
        }
        // Host trap: most commonly the token address has no deployed code.
        Err(Err(InvokeError::Abort)) => Err(Error::TokenMissing),
    }
}

/// Pull `amount` of `token` from `from` into the contract's own balance and
/// verify the pool grew by exactly `amount`.
///
/// # Why measure rather than trust the requested amount
///
/// Fluxora's accounting — [`Stream::deposited`], and every rate, liability and
/// refund figure derived from it — assumes a deposit pull moves exactly the
/// amount requested into the pool. A **fee-on-transfer** token (a cut taken
/// out of every transfer) silently breaks that assumption: the sender's
/// balance drops by `amount`, but the pool only grows by `amount` minus the
/// fee. Nothing about that failure is loud — the deposit "succeeds" and the
/// shortfall says nothing until, much later, an unrelated stream's `withdraw`
/// or `cancel` fails with [`Error::TokenTransferFailed`] because the pool
/// cannot cover every stream's claim. See `docs/ABI.md` "Token assumptions".
///
/// The fix is to check, not assume: read the contract's own balance before
/// and after the pull and require the delta to equal `amount` exactly. Any
/// deviation — a shortfall from a fee, or an overage from a positive-rebasing
/// token — is rejected with [`Error::TokenAmountMismatch`]. Soroban rolls back
/// the entire invocation on error, so the transfer that already happened
/// (and any fee it took) is undone along with everything else; nothing is
/// stranded.
///
/// # Why this guards deposits only
///
/// The outbound legs — [`FluxoraStream::withdraw`]'s payout and
/// [`FluxoraStream::cancel`]'s refund — call [`token_transfer`] directly, with
/// no balance-delta check. If the token takes a further cut on receipt there,
/// that is between the recipient (or sender) and their own balance: the *pool's*
/// balance still drops by exactly the amount the contract sent, so Fluxora's
/// internal accounting stays in sync either way.
///
/// What *is* checked on the outbound legs is the pool total itself, by
/// [`verify_pool_balance`] — see [`storage::PooledBalance`] and
/// `docs/KNOWN-LIMITATIONS.md` §6.
fn pull_deposit(env: &Env, token: &Address, from: &Address, amount: &i128) -> Result<(), Error> {
    let contract = env.current_contract_address();
    let token_client = token::TokenClient::new(env, token);
    let before = token_client.balance(&contract);

    token_transfer(
        env,
        token,
        from,
        MuxedAddress::from(contract.clone()),
        amount,
    )?;

    let after = token_client.balance(&contract);
    let received = after.checked_sub(before).ok_or(Error::Overflow)?;
    if received != *amount {
        return Err(Error::TokenAmountMismatch);
    }

    // The pull is verified, so the pool now holds exactly `amount` more for
    // this token. Record it — this running total is what later operations
    // reconcile against.
    storage::credit_pool(env, token, *amount)?;
    Ok(())
}

/// Reconcile the pool's real balance for `token` against the balance Fluxora
/// has accounted for, rejecting a shortfall with [`Error::PoolBalanceDrift`].
///
/// # Why a running total rather than a sum over streams
///
/// The expected balance is maintained incrementally in
/// [`DataKey::PooledBalance`]: every verified pull credits it, every payout
/// and refund debits it. Deriving it instead — walking every stream at call
/// time to sum `deposited - withdrawn` — is impossible here: the contract has
/// no index of which streams hold which token, so the walk would need the
/// whole stream population, which neither fits an invocation budget nor
/// survives an archived entry. An incremental total is O(1) per call and, being
/// instance storage, cannot archive while a stream does.
///
/// # Why a shortfall is checked and a surplus is not
///
/// `actual < expected` means the token destroyed value the pool was counting
/// on: an elastic-supply rebase, or any balance move Fluxora was not a party
/// to. Every outstanding claim is now backed by less than the accounting says,
/// so paying any one of them out of the pool spends another stream's money —
/// the invocation is rejected instead.
///
/// `actual > expected` is deliberately **accepted**. A positive rebase cannot
/// cause an underpayment, and the excess is inert: payouts are sized by stream
/// accounting, never by the pool balance, so the surplus simply sits there.
/// Requiring equality would hand any third party a one-unit griefing stick —
/// `transfer` a single stroop into the contract and every withdrawal in the
/// protocol would revert with a drift error. A solvency check must not be
/// triggerable by anyone who is not even a party to a stream.
///
/// # Where it is called
///
/// At the end of every operation that moves pool funds: after the payout in
/// [`FluxoraStream::withdraw`] / [`delegate_withdraw`] / [`batch_withdraw`],
/// after the refund in [`FluxoraStream::cancel`] / [`delegate_cancel`], and
/// after the pull in [`FluxoraStream::top_up`] / [`delegate_top_up`]. So a
/// rebase that happens between two operations is detected by the *next* one,
/// which reverts in full.
fn verify_pool_balance(env: &Env, token: &Address) -> Result<(), Error> {
    let expected = storage::pooled_balance(env, token);
    let actual = token::TokenClient::new(env, token).balance(&env.current_contract_address());
    if actual < expected {
        return Err(Error::PoolBalanceDrift);
    }
    Ok(())
}

#[contract]
pub struct FluxoraStream;

#[contractimpl]
impl FluxoraStream {
    // ---------------------------------------------------------------------
    // Lifecycle
    // ---------------------------------------------------------------------

    /// Create a stream and move `deposit` from `sender` into the contract's
    /// pooled balance.
    ///
    /// Returns the new stream id. The id is monotonic and never reused, so it is
    /// a stable handle for an indexer.
    ///
    /// This is [`CliffMode::Schedule`]: the cliff is a point on the stream
    /// clock, so pausing a `pausable` stream before its cliff defers the gate.
    /// Callers who need a cliff that pausing cannot move — a contractual date —
    /// want [`create_stream_with_cliff_mode`](Self::create_stream_with_cliff_mode).
    ///
    /// Every parameter, the accrual semantics, and the error set are otherwise
    /// identical between the two entry points; this one delegates.
    #[allow(clippy::too_many_arguments)]
    pub fn create_stream(
        env: Env,
        sender: Address,
        recipient: Address,
        token: Address,
        deposit: i128,
        start_time: u64,
        end_time: u64,
        cliff_time: u64,
        cancellable: bool,
        pausable: bool,
        transferable: bool,
        reference: Option<String>,
    ) -> Result<u64, Error> {
        Self::create_stream_with_cliff_mode(
            env,
            sender,
            recipient,
            token,
            deposit,
            start_time,
            end_time,
            cliff_time,
            CliffMode::DEFAULT,
            cancellable,
            pausable,
            transferable,
            reference,
        )
    }

    /// Create a stream, choosing which clock the cliff gate is read against.
    ///
    /// Returns the new stream id. The id is monotonic and never reused, so it is
    /// a stable handle for an indexer.
    ///
    /// Identical to [`create_stream`](Self::create_stream) except for the
    /// `cliff_mode` parameter. Prefer this one when the caller has an opinion
    /// about pausing; prefer `create_stream` when it does not, since
    /// [`CliffMode::Schedule`] is the default and the long-standing behaviour.
    ///
    /// # Schedule
    ///
    /// Tokens accrue linearly from `start_time` to `end_time` — the
    /// [`ReleaseCurve::Linear`] default. Use
    /// [`create_stream_with_curve`](FluxoraStream::create_stream_with_curve) to
    /// select a different release shape; this entry point is kept signature- and
    /// arithmetic-identical so an existing integrator sees no change at all.
    /// `start_time` may
    /// be in the past — backdated vesting from a hire date or grant award date
    /// is a legitimate use — in which case the backdated portion is immediately
    /// withdrawable. It may equally be in the future — a scheduled stream — in
    /// which case nothing vests until the start instant.
    ///
    /// There is deliberately **no bound on clock skew** in either direction.
    /// The ledger timestamp is the only clock the contract can see, so a skew
    /// limit would be an arbitrary business-policy number rather than a
    /// protocol requirement; policy belongs in the SDK or a wrapping contract.
    /// A past start vests immediately by the sender's own authorization, and a
    /// schedule that has already fully elapsed simply reads as fully vested.
    /// The accrual math is safe at any skew — every quantity clamps or is
    /// checked — and TTL is where the real risk is bounded: a stream whose
    /// schedule extends beyond the network's `max_entry_ttl` horizon is funded
    /// for as long as the network allows at creation, and the permissionless
    /// `extend_stream_ttl` keeper path covers the remainder, exactly as it
    /// does for any multi-year stream. The regression tests in `test/create.rs`
    /// pin these semantics.
    ///
    /// # The cliff gate
    ///
    /// `cliff_time` **gates** the payout, it does not delay accrual. Pass
    /// `cliff_time == start_time` for no cliff. At the cliff instant the
    /// recipient becomes entitled to everything accrued since `start_time`, not
    /// merely what accrues after the cliff. This is standard vesting semantics
    /// and it surprises people, so it is worth restating in any UI.
    ///
    /// `cliff_mode` decides which clock that instant is read against, and so
    /// whether pausing can move it. `cliff_time` is validated against
    /// `[start_time, end_time]` in both modes, and is fixed at creation either
    /// way.
    ///
    /// * [`CliffMode::Schedule`] (the default) — gate at `cliff_time` on the
    ///   stream clock, so pausing freezes it and resuming pushes the wall-clock
    ///   opening instant forward by the total paused duration.
    /// * [`CliffMode::WallClock`] — gate at `cliff_time` on the ledger clock.
    ///   Pausing never moves it. Pausing still stops *accrual*, so a stream
    ///   paused before its cliff opens the gate on schedule and pays out only
    ///   what had accrued when it was paused.
    ///
    /// The mode is a term of the stream, fixed at creation and never mutable —
    /// the same trust property as `cancellable` / `pausable` / `transferable`.
    /// A recipient can therefore read `cliff_mode` and know exactly which
    /// reading of `cliff_time` applies. It is published by
    /// `stream_created` and readable from `get_stream`.
    ///
    /// Choose `WallClock` when `cliff_time` is a contractual date the recipient
    /// is entitled to hold you to; choose `Schedule` when the cliff is a
    /// milestone in your own schedule and stretching it alongside the schedule
    /// is the intent. Neither mode changes the total value delivered.
    ///
    /// # Errors
    ///
    /// * [`Error::SelfStream`] — sender and recipient are the same address.
    /// * [`Error::InvalidDeposit`] — deposit is not positive.
    /// * [`Error::InvalidTimeRange`] — `end_time <= start_time`.
    /// * [`Error::DurationTooLong`] — `end_time - start_time` exceeds
    ///   [`MAX_STREAM_DURATION`].
    ///
    ///   **Zero-duration design decision:** a stream with `end_time == start_time`
    ///   (or earlier) is **rejected**, never treated as "already vested". A zero
    ///   length would make every accrual formula divide by zero, and there is no
    ///   meaningful schedule for a single-instant stream to vest against. The
    ///   one legitimate zero-length state — a *cancel* that collapses a live
    ///   schedule onto its start instant — is produced by [`FluxoraStream::cancel`]
    ///   and handled specially in [`vested`], which returns the settled deposit
    ///   in full rather than dividing.
    /// * [`Error::InvalidCliff`] — cliff outside `[start_time, end_time]`.
    /// * [`Error::DepositRateTooLow`] — `deposit < duration`, so the per-second
    ///   rate would truncate to zero and the recipient would accrue nothing.
    /// * [`Error::Overflow`] — `deposit * duration` does not fit in `i128`.
    /// * [`Error::TokenAmountMismatch`] — the deposit pull delivered a
    ///   different amount than `deposit` (a fee-on-transfer or rebasing
    ///   token). See `docs/ABI.md` "Token assumptions".
    #[allow(clippy::too_many_arguments)]
    pub fn create_stream_with_cliff_mode(
        env: Env,
        sender: Address,
        recipient: Address,
        token: Address,
        deposit: i128,
        start_time: u64,
        end_time: u64,
        cliff_time: u64,
        cliff_mode: CliffMode,
        cancellable: bool,
        pausable: bool,
        transferable: bool,
        reference: Option<String>,
    ) -> Result<u64, Error> {
        Self::require_not_halted(&env)?;
        sender.require_auth();

        Self::create_stream_inner(
            env,
            sender,
            recipient,
            token,
            deposit,
            start_time,
            end_time,
            cliff_time,
            cliff_mode,
            cancellable,
            pausable,
            transferable,
            reference,
            ReleaseCurve::Linear,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn create_stream_unchecked(
        env: Env,
        sender: Address,
        recipient: Address,
        token: Address,
        deposit: i128,
        start_time: u64,
        end_time: u64,
        cliff_time: u64,
        cliff_mode: CliffMode,
        cancellable: bool,
        pausable: bool,
        transferable: bool,
        reference: Option<String>,
    ) -> Result<u64, Error> {
        Self::create_stream_inner(
            env,
            sender,
            recipient,
            token,
            deposit,
            start_time,
            end_time,
            cliff_time,
            cliff_mode,
            cancellable,
            pausable,
            transferable,
            reference,
            ReleaseCurve::Linear,
        )
    }

    /// Create a stream with an explicit [`ReleaseCurve`].
    ///
    /// Identical to [`create_stream`](FluxoraStream::create_stream) in every
    /// respect — same authorization, same validation, same deposit pull, same
    /// event — except that the shape of the release schedule above the cliff is
    /// chosen by `curve` instead of defaulting to [`ReleaseCurve::Linear`].
    ///
    /// # Curve semantics
    ///
    /// All curves are interchangeable in *total*: each is monotone
    /// non-decreasing on the stream clock and each delivers exactly `deposited`
    /// by `end_time`. That common contract is what keeps the pool solvent —
    /// `vested + refundable == deposited` holds at every instant, for every
    /// curve — so the choice is purely about *when* the recipient's claim
    /// grows, never about how much they eventually receive.
    ///
    /// * [`ReleaseCurve::Linear`] — `floor(deposited * elapsed / duration)`,
    ///   byte-identical to [`create_stream`](FluxoraStream::create_stream).
    /// * [`ReleaseCurve::Step`] — four equal tranches, opening at 25%, 50%,
    ///   75% and 100% of the schedule.
    /// * [`ReleaseCurve::FrontLoaded`] — `f(u) = 2u - u²`, ahead of linear for
    ///   the whole schedule and settling at the same endpoint.
    ///
    /// The curve is fixed at creation and can never change, exactly like
    /// `cancellable`, `pausable` and `transferable`. It is returned by
    /// [`get_stream`](FluxoraStream::get_stream) and carried on the
    /// `StreamCreated` event so an off-chain indexer can see it without a
    /// second call.
    ///
    /// The same `deposit` / `duration` sanity guards apply as for
    /// [`create_stream`](FluxoraStream::create_stream), including
    /// [`Error::DepositRateTooLow`]; the front-loaded overflow guard it
    /// establishes bounds `deposited * duration` (with the curve reading
    /// `<= duration`), so no curve can overflow the accrual arithmetic later.
    ///
    /// # Errors
    ///
    /// The error set is identical to
    /// [`create_stream`](FluxoraStream::create_stream); `curve` is always one
    /// of the three variants above and is never rejected.
    #[allow(clippy::too_many_arguments)]
    pub fn create_stream_with_curve(
        env: Env,
        sender: Address,
        recipient: Address,
        token: Address,
        deposit: i128,
        start_time: u64,
        end_time: u64,
        cliff_time: u64,
        cancellable: bool,
        pausable: bool,
        transferable: bool,
        curve: ReleaseCurve,
    ) -> Result<u64, Error> {
        Self::require_not_halted(&env)?;
        sender.require_auth();

        Self::create_stream_inner(
            env,
            sender,
            recipient,
            token,
            deposit,
            start_time,
            end_time,
            cliff_time,
            CliffMode::DEFAULT,
            cancellable,
            pausable,
            transferable,
            None,
            curve,
        )
    }

    /// The body shared by [`create_stream`](FluxoraStream::create_stream) and
    /// [`create_stream_with_curve`](FluxoraStream::create_stream_with_curve).
    ///
    /// Kept private (and therefore not exported as an ABI entry point) so the
    /// two public entry points are the single, documented surface and neither
    /// can drift from the other: the linear path *is* the curve path with
    /// [`ReleaseCurve::Linear`] selected.
    #[allow(clippy::too_many_arguments)]
    fn create_stream_inner(
        env: Env,
        sender: Address,
        recipient: Address,
        token: Address,
        deposit: i128,
        start_time: u64,
        end_time: u64,
        cliff_time: u64,
        cliff_mode: CliffMode,
        cancellable: bool,
        pausable: bool,
        transferable: bool,
        reference: Option<String>,
        curve: ReleaseCurve,
    ) -> Result<u64, Error> {
        if sender == recipient {
            return Err(Error::SelfStream);
        }
        if deposit <= 0 {
            return Err(Error::InvalidDeposit);
        }
        if end_time <= start_time {
            return Err(Error::InvalidTimeRange);
        }
        let total_duration = end_time - start_time;
        if total_duration > MAX_STREAM_DURATION {
            return Err(Error::DurationTooLong);
        }
        if cliff_time < start_time || cliff_time > end_time {
            return Err(Error::InvalidCliff);
        }

        // Validate reference length if provided
        if let Some(ref r) = reference {
            if r.len() > MAX_REFERENCE_LENGTH {
                return Err(Error::InvalidReferenceLength);
            }
        }

        // Reject dust-rate streams. Below one stroop per second the recipient
        // accrues literally nothing until very late in the schedule, which is a
        // real footgun for a treasury streaming a small grant over a year.
        if deposit < total_duration as i128 {
            return Err(Error::DepositRateTooLow);
        }

        // Front-load the overflow guard for all future accrual. Because
        // `elapsed <= duration` always holds, proving `deposit * duration` fits
        // in an i128 here means the `deposited * elapsed` multiplication inside
        // `vested` can never overflow for the life of the stream. `top_up`
        // re-establishes the same guard against its new figures.
        deposit
            .checked_mul(total_duration as i128)
            .ok_or(Error::Overflow)?;

        let stream_id = storage::next_stream_id(&env)?;
        let stream = Stream {
            sender: sender.clone(),
            recipient,
            token: token.clone(),
            deposited: deposit,
            withdrawn: 0,
            start_time,
            end_time,
            cliff_time,
            cliff_mode,
            cancellable,
            pausable,
            transferable,
            paused_at: None,
            paused_total: 0,
            status: StreamStatus::Active,
            curve,
            reference,
        };

        // Pull the deposit before writing the stream entry. If the token
        // transfer fails (missing contract, authorization refused, insufficient
        // balance) we return a typed error and leave no phantom entry in
        // storage — the id counter has already advanced, so the id is
        // consumed, but no stream with that id is observable to any caller.
        //
        // The sender's auth on this invocation covers the nested token
        // transfer; no prior approval is needed.
        pull_deposit(&env, &token, &sender, &deposit)?;

        storage::save_stream(&env, stream_id, &stream);
        storage::extend_instance(&env);

        events::stream_created(&env, stream_id, &stream);
        Ok(stream_id)
    }

    /// Create several streams atomically for one sender.
    ///
    /// All requests are validated and submitted in this invocation. Soroban
    /// rolls back the complete invocation if any element fails, so a failed
    /// transfer cannot leave a partially-created payroll. IDs are returned in
    /// the same order as the input requests.
    pub fn batch_create(
        env: Env,
        sender: Address,
        requests: Vec<BatchCreateRequest>,
    ) -> Result<Vec<u64>, Error> {
        Self::require_not_halted(&env)?;
        if requests.is_empty() {
            return Err(Error::EmptyBatch);
        }
        if requests.len() > MAX_BATCH_SIZE {
            return Err(Error::BatchTooLarge);
        }
        sender.require_auth();

        // Validate the complete batch before the first token call. This makes
        // malformed payroll input fail before any external transfer is tried;
        // the transaction-level rollback still guarantees all-or-none when a
        // later token transfer fails.
        for request in requests.iter() {
            if request.recipient == sender {
                return Err(Error::SelfStream);
            }
            if request.deposit <= 0 {
                return Err(Error::InvalidDeposit);
            }
            if request.end_time <= request.start_time {
                return Err(Error::InvalidTimeRange);
            }
            if request.end_time - request.start_time > MAX_STREAM_DURATION {
                return Err(Error::DurationTooLong);
            }
            if request.cliff_time < request.start_time || request.cliff_time > request.end_time {
                return Err(Error::InvalidCliff);
            }
            if request.deposit < (request.end_time - request.start_time) as i128 {
                return Err(Error::DepositRateTooLow);
            }
        }

        let mut ids = Vec::new(&env);
        for request in requests.iter() {
            ids.push_back(Self::create_stream_unchecked(
                env.clone(),
                sender.clone(),
                request.recipient.clone(),
                request.token.clone(),
                request.deposit,
                request.start_time,
                request.end_time,
                request.cliff_time,
                CliffMode::DEFAULT,
                request.cancellable,
                request.pausable,
                request.transferable,
                None,
            )?);
        }
        Ok(ids)
    }

    /// Create a stream enforcing the policy of a deployed [`FluxoraFactory`].
    ///
    /// Identical to [`create_stream`](Self::create_stream) in every respect
    /// except that it first loads the factory's policy via a cross-contract
    /// call and validates the request against all configured constraints before
    /// creating the stream:
    ///
    /// * **Pause check** — if the factory's creation pause is on, the call
    ///   returns [`Error::FactoryPaused`] immediately.
    /// * **Deposit cap** — `deposit` must not exceed `policy.max_deposit`;
    ///   excess returns [`Error::DepositExceedsCap`].
    /// * **Duration floor** — `end_time - start_time` must be at least
    ///   `policy.min_duration`; a shorter schedule returns
    ///   [`Error::DurationBelowMinimum`].
    /// * **Token allowlist** — the `token` must be allowlisted on the factory;
    ///   an absent entry returns [`Error::TokenNotAllowlisted`].
    /// * **Rate bounds** — if set, the per-second rate (`deposit / duration`)
    ///   must lie within `[min_rate_per_second, max_rate_per_second]`; a rate
    ///   outside the interval returns [`Error::RateBelowMin`] or
    ///   [`Error::RateAboveMax`].
    ///
    /// All other validation (self-stream, dust rate, overflow guard, etc.) is
    /// identical to [`create_stream`](Self::create_stream).
    ///
    /// # Authorization
    ///
    /// Requires `sender`'s auth, exactly as [`create_stream`](Self::create_stream).
    /// No factory-admin auth is needed; the factory contract itself is read
    /// via permissionless view calls.
    ///
    /// # Errors
    ///
    /// All errors from [`create_stream`](Self::create_stream) plus:
    /// * [`Error::FactoryPaused`]
    /// * [`Error::DepositExceedsCap`]
    /// * [`Error::DurationBelowMinimum`]
    /// * [`Error::TokenNotAllowlisted`]
    /// * [`Error::RateBelowMin`]
    /// * [`Error::RateAboveMax`]
    #[allow(clippy::too_many_arguments)]
    pub fn create_stream_via_factory(
        env: Env,
        factory: Address,
        sender: Address,
        recipient: Address,
        token: Address,
        deposit: i128,
        start_time: u64,
        end_time: u64,
        cliff_time: u64,
        cancellable: bool,
        pausable: bool,
        transferable: bool,
    ) -> Result<u64, Error> {
        // Load policy from the factory contract via cross-contract calls.
        // `get_factory_config` and `is_allowlisted` are permissionless read
        // views — no auth is required or consumed.
        use soroban_sdk::Symbol;

        // Fetch the full config in one call.
        let config: FactoryConfig = env.invoke_contract(
            &factory,
            &Symbol::new(&env, "get_factory_config"),
            soroban_sdk::vec![&env],
        );

        // 1. Pause check — refuse first, before any other work.
        if config.creation_paused {
            return Err(Error::FactoryPaused);
        }

        // 2. Deposit cap.
        if deposit > config.max_deposit {
            return Err(Error::DepositExceedsCap);
        }

        // 3. Duration floor — validate time range first so the subtraction
        //    cannot underflow (end_time > start_time is checked inside
        //    create_stream_inner, but we need the duration here).
        if end_time <= start_time {
            // Let create_stream_inner produce the proper InvalidTimeRange error.
        } else {
            let duration = end_time - start_time;
            if duration < config.min_duration {
                return Err(Error::DurationBelowMinimum);
            }

            // 4. Rate bounds — rate = deposit / duration (integer floor).
            //    deposit <= 0 is caught by create_stream_inner; a non-positive
            //    deposit here simply skips the check, letting the inner path
            //    produce InvalidDeposit.
            if deposit > 0 {
                let rate = deposit / duration as i128;
                if let Some(min_rate) = config.min_rate_per_second {
                    if rate < min_rate {
                        return Err(Error::RateBelowMin);
                    }
                }
                if let Some(max_rate) = config.max_rate_per_second {
                    if rate > max_rate {
                        return Err(Error::RateAboveMax);
                    }
                }
            }
        }

        // 5. Token allowlist.
        let allowlisted: bool = env.invoke_contract(
            &factory,
            &Symbol::new(&env, "is_allowlisted"),
            soroban_sdk::vec![&env, token.to_val()],
        );
        if !allowlisted {
            return Err(Error::TokenNotAllowlisted);
        }

        // All policy checks passed — create the stream normally.
        Self::create_stream_inner(
            env,
            sender,
            recipient,
            token,
            deposit,
            start_time,
            end_time,
            cliff_time,
            CliffMode::DEFAULT,
            cancellable,
            pausable,
            transferable,
            None, // reference
            ReleaseCurve::Linear,
        )
    }

    /// Add funds to a live stream.
    ///
    /// # Semantics: extend the duration, keep the rate
    ///
    /// The per-second rate the recipient agreed to at creation **never
    /// changes**. `end_time` moves forward by `amount / rate` so the added
    /// tokens stream out at the original pace:
    ///
    /// ```text
    /// before:  10_000 over 100 days  ->  100/day, ends day 100
    /// top_up(1_000)
    /// after:   11_000 over 110 days  ->  100/day, ends day 110
    /// ```
    ///
    /// The alternative — hold `end_time` and raise the rate — was rejected
    /// because it retroactively re-vests elapsed time: a top-up at the halfway
    /// point would instantly increase the amount already withdrawable. Keeping
    /// the rate fixed means a top-up can never accelerate or dilute an existing
    /// schedule, which is the property that makes it safe to accept a stream
    /// from an untrusted sender.
    ///
    /// # Rounding
    ///
    /// The duration extension rounds **down**. That direction is load-bearing,
    /// not cosmetic: rounding up would make the new duration slightly longer
    /// than exact, which lowers the rate and therefore *retroactively reduces*
    /// the amount already vested. A recipient who had withdrawn at the old rate
    /// would then hold more than `vested`, and a subsequent `cancel` — which
    /// sets `deposited = vested` — would drive the stream's liability negative
    /// and refund the sender money the recipient already has.
    ///
    /// Rounding down guarantees `vested` never decreases across a top-up. The
    /// residual is at most one second of schedule, in the recipient's favour.
    ///
    /// # Release curve
    ///
    /// The rate-preserving extension above is the [`ReleaseCurve::Linear`] rule.
    /// A [`ReleaseCurve::Step`] or [`ReleaseCurve::FrontLoaded`] stream has no
    /// constant per-second rate to preserve — that is the point of the curve —
    /// so `top_up` takes the other safe option for those: it keeps `end_time`
    /// and lets the added deposit ride the same curve, scaled proportionally.
    /// See `top_up_duration_delta`. Either way the property that matters is
    /// unchanged: `vested` never decreases across a top-up, and `top_up` still
    /// re-checks it before committing (`Error::VestedDecreased`).
    ///
    /// # Errors
    ///
    /// * [`Error::StreamMatured`] — the accrual clock has already reached
    ///   `end_time`. Extending a matured stream would make the new funds
    ///   instantly (or near-instantly) withdrawable, which is never what the
    ///   sender means. Create a new stream instead.
    /// * [`Error::StreamTerminated`] — stream is cancelled or depleted.
    /// * [`Error::DurationTooLong`] — extending the schedule would exceed
    ///   [`MAX_STREAM_DURATION`].
    /// * [`Error::TokenAmountMismatch`] — the pull delivered a different
    ///   amount than `amount` (a fee-on-transfer or rebasing token). See
    ///   `docs/ABI.md` "Token assumptions".
    pub fn top_up(env: Env, stream_id: u64, amount: i128) -> Result<(), Error> {
        // Emergency halt (#1818): refuse state changes before anything else.
        Self::require_not_halted(&env)?;
        let mut stream = storage::load_stream(&env, stream_id)?;
        stream.sender.require_auth();

        if stream.status.is_terminal() {
            return Err(Error::StreamTerminated);
        }
        if amount <= 0 {
            return Err(Error::InvalidAmount);
        }

        let now = env.ledger().timestamp();
        if accrual::stream_time(&stream, now) >= stream.end_time {
            return Err(Error::StreamMatured);
        }

        let current_duration = accrual::duration(&stream) as i128;

        // How many seconds the schedule must grow by so the top-up is absorbed
        // without retroactively re-vesting elapsed time. Curve-specific; see
        // `top_up_duration_delta`.
        let delta = Self::top_up_duration_delta(&stream, amount, current_duration)?;

        let new_deposited = stream
            .deposited
            .checked_add(amount)
            .ok_or(Error::Overflow)?;
        let new_end = stream
            .end_time
            .checked_add(delta as u64)
            .ok_or(Error::Overflow)?;
        let new_duration = new_end
            .checked_sub(stream.start_time)
            .ok_or(Error::Overflow)?;
        if new_duration > MAX_STREAM_DURATION {
            return Err(Error::DurationTooLong);
        }

        // Re-establish the creation-time guards against the new figures.
        new_deposited
            .checked_mul(new_duration as i128)
            .ok_or(Error::Overflow)?;
        if new_deposited < new_duration as i128 {
            return Err(Error::DepositRateTooLow);
        }

        let old_vested = accrual::vested(&stream, now)?;
        stream.deposited = new_deposited;
        stream.end_time = new_end;
        if accrual::vested(&stream, now)? < old_vested {
            return Err(Error::VestedDecreased);
        }

        let token = stream.token.clone();
        let sender = stream.sender.clone();

        pull_deposit(&env, &token, &sender, &amount)?;
        // Reconcile the token's pool total now that the pull has landed. A
        // rebase since the last operation on this token shows up as a
        // shortfall and rolls the top-up back (Error::PoolBalanceDrift).
        verify_pool_balance(&env, &token)?;

        storage::save_stream(&env, stream_id, &stream);

        events::topped_up(&env, stream_id, &stream, amount);
        Ok(())
    }

    /// Seconds by which a top-up extends the schedule, for the stream's curve.
    ///
    /// [`ReleaseCurve::Linear`] is the original rule: `floor(amount * duration /
    /// deposited)`, keeping the per-second rate fixed — see the rounding note on
    /// [`FluxoraStream::top_up`] for why it floors and why a zero delta is
    /// rejected rather than absorbed.
    ///
    /// [`ReleaseCurve::Step`] and [`ReleaseCurve::FrontLoaded`] have no constant
    /// per-second rate to hold fixed, so `end_time` is left alone and the added
    /// deposit rides the same curve. `vested` is monotone in `deposited` for a
    /// fixed curve, so this can only raise the recipient's claim, which
    /// [`FluxoraStream::top_up`] re-checks before committing.
    fn top_up_duration_delta(
        stream: &Stream,
        amount: i128,
        current_duration: i128,
    ) -> Result<i128, Error> {
        match stream.curve {
            ReleaseCurve::Linear => {
                // delta = floor(amount * duration / deposited), preserving the
                // rate. Floor, never ceiling — see the rounding note above.
                let scaled = amount
                    .checked_mul(current_duration)
                    .ok_or(Error::Overflow)?;
                let delta = scaled
                    .checked_div(stream.deposited)
                    .ok_or(Error::Overflow)?;
                if delta < 0 || delta > u64::MAX as i128 {
                    return Err(Error::Overflow);
                }
                // A top-up too small to buy even one second cannot extend the
                // schedule, so the only way to absorb it would be to raise the
                // rate — which re-vests elapsed time retroactively, the exact
                // thing this function exists to avoid. Reject instead.
                if delta == 0 {
                    return Err(Error::TopUpTooSmall);
                }
                Ok(delta)
            }
            // No rate to preserve: hold the schedule, scale the deposit in place.
            ReleaseCurve::Step | ReleaseCurve::FrontLoaded => Ok(0),
        }
    }

    /// Withdraw accrued tokens to the recipient.
    ///
    /// `amount == None` withdraws the full withdrawable balance. Returns the
    /// amount actually transferred.
    ///
    /// Withdrawal works while the stream is paused: pausing stops *accrual*, it
    /// does not freeze funds the recipient has already earned. Freezing earned
    /// funds would make pausable streams unacceptable to any serious recipient.
    ///
    /// # Errors
    ///
    /// * [`Error::StreamNotFound`] — no stream with this id.
    /// * [`Error::StreamTerminated`] — stream is `Cancelled` or `Depleted` and
    ///   has nothing left to pay. Distinct from [`Error::NothingToWithdraw`] so
    ///   a client can tell "wait for accrual" apart from "this stream is over"
    ///   without a second round-trip.
    /// * [`Error::NothingToWithdraw`] — stream is still live but the
    ///   withdrawable balance is zero (pre-start, pre-cliff, or fully drawn
    ///   for now). A typed error rather than a silent no-op.
    /// * [`Error::InsufficientWithdrawable`] — explicit amount exceeds the
    ///   withdrawable balance.
    pub fn withdraw(env: Env, stream_id: u64, amount: Option<i128>) -> Result<i128, Error> {
        // Emergency halt (#1818): refuse state changes before anything else.
        Self::require_not_halted(&env)?;
        let mut stream = storage::load_stream(&env, stream_id)?;
        stream.recipient.require_auth();

        let now = env.ledger().timestamp();
        let available = accrual::withdrawable(&stream, now)?;
        if available == 0 {
            // Terminal with nothing left is a different precondition from a
            // live stream that simply has not accrued (or is fully drawn for
            // now). Integrators must be able to branch without guessing.
            if stream.status.is_terminal() {
                return Err(Error::StreamTerminated);
            }
            return Err(Error::NothingToWithdraw);
        }

        let payout = match amount {
            None => available,
            Some(requested) => {
                if requested <= 0 {
                    return Err(Error::InvalidAmount);
                }
                if requested > available {
                    return Err(Error::InsufficientWithdrawable);
                }
                requested
            }
        };

        Self::apply_withdrawal(&env, stream_id, &mut stream, payout)?;
        // Reconcile the pool against the token's real balance now the payout
        // has left it. A rebase between this token's last operation and this
        // one shows up as a shortfall and rolls the withdrawal back
        // (`Error::PoolBalanceDrift`) instead of letting the pool drift further.
        verify_pool_balance(&env, &stream.token)?;
        Ok(payout)
    }

    /// Withdraw the full available balance from several streams at once.
    ///
    /// All streams must share the same `recipient`, who authorizes once for the
    /// whole batch. Streams with nothing currently withdrawable are skipped
    /// rather than failing the batch. Returns the total transferred across all
    /// streams; per-stream amounts are available from the individual `withdrawn`
    /// events, which are emitted in batch order.
    ///
    /// Streams need not share a token — each payout uses its own stream's token.
    ///
    /// **Atomicity: the batch is all-or-nothing.** Any error — an unknown id, a
    /// stream belonging to a different recipient, or a duplicate id — reverts
    /// the *entire* call, including payouts already applied to earlier streams
    /// in the batch. No accounting is written, no tokens move, and no event is
    /// observable. A failed batch leaves the caller free to retry with a
    /// corrected id list; the duplicates are rejected deterministically
    /// ([`Error::DuplicateStreamId`]) no matter where in the batch they sit.
    ///
    /// # Errors
    ///
    /// * [`Error::EmptyBatch`] — no ids were supplied.
    /// * [`Error::BatchTooLarge`] — more than [`MAX_BATCH_SIZE`] ids. Chunk
    ///   client-side; the SDK does this automatically.
    /// * [`Error::MalformedStreamId`] — a serialized vector element is not a
    ///   `u64`.
    /// * [`Error::DuplicateStreamId`] — the same id appears twice, which would
    ///   otherwise operate on a stale copy of the stream the second time.
    /// * [`Error::StreamNotFound`] — one of the ids does not exist.
    /// * [`Error::Unauthorized`] — one of the streams has a different recipient.
    pub fn batch_withdraw(
        env: Env,
        recipient: Address,
        stream_ids: Vec<u64>,
    ) -> Result<i128, Error> {
        // Emergency halt (#1818): refuse state changes before anything else.
        Self::require_not_halted(&env)?;
        let stream_ids = Self::validate_batch_ids(&env, &stream_ids)?;
        Self::reject_duplicate_ids(&stream_ids)?;
        recipient.require_auth();

        let now = env.ledger().timestamp();
        let mut streams = Vec::new(&env);
        let mut payouts = Vec::new(&env);
        let mut total: i128 = 0;

        // Resolve and validate the entire batch before changing storage or
        // calling any token contract.
        for stream_id in stream_ids.iter() {
            let stream = storage::peek_stream(&env, stream_id)?;
            if stream.recipient != recipient {
                return Err(Error::Unauthorized);
            }

            let available = accrual::withdrawable(&stream, now)?;
            total = total.checked_add(available).ok_or(Error::Overflow)?;
            streams.push_back(stream);
            payouts.push_back(available);
        }

        for i in 0..stream_ids.len() {
            let stream_id = stream_ids.get_unchecked(i);
            let mut stream = streams.get_unchecked(i);
            let payout = payouts.get_unchecked(i);
            if payout == 0 {
                storage::extend_stream(&env, stream_id, &stream);
            } else {
                Self::apply_withdrawal(&env, stream_id, &mut stream, payout)?;
            }
        }

        // Reconcile every token the batch touched, now that all payouts have
        // left the pool. Deduplicated by token: a batch may hold several
        // streams on one token (the payroll case), and one `balance`
        // sub-invocation per stream would spend the instruction budget on
        // repeated answers to the same question.
        let mut reconciled: Vec<Address> = Vec::new(&env);
        for stream in streams.iter() {
            let token = stream.token.clone();
            if !reconciled.contains(&token) {
                verify_pool_balance(&env, &token)?;
                reconciled.push_back(token);
            }
        }

        Ok(total)
    }

    /// Cancel a stream: stop accrual and refund the unvested remainder to the
    /// sender.
    ///
    /// The recipient keeps everything vested up to this instant and withdraws it
    /// through the normal path — cancellation does not seize earned funds.
    ///
    /// # Implementation
    ///
    /// Rather than introduce a second state machine, cancellation rewrites the
    /// schedule so the stream *looks* like one that has fully matured:
    /// `deposited` is reduced to the amount vested right now and `end_time` is
    /// pulled back to the current point on the stream clock. Every subsequent
    /// `vested` call then clamps to the full (reduced) deposit, so
    /// [`withdraw`](Self::withdraw) needs no special-casing at all.
    ///
    /// Cancelling before the cliff refunds everything: pre-cliff the recipient's
    /// entitlement is zero by definition.
    ///
    /// Cancelling at exactly `start_time` is that case one instant later:
    /// nothing has vested, so the entire deposit is returned, the recipient
    /// receives nothing, and the collapsed schedule has zero length rather than
    /// a negative one.
    ///
    /// # Errors
    ///
    /// * [`Error::StreamNotFound`] — no stream with this id.
    /// * [`Error::NotCancellable`] — created with `cancellable == false`.
    /// * [`Error::StreamTerminated`] — already cancelled or depleted.
    pub fn cancel(env: Env, stream_id: u64) -> Result<(), Error> {
        // Emergency halt (#1818): refuse state changes before anything else.
        Self::require_not_halted(&env)?;
        let mut stream = storage::load_stream(&env, stream_id)?;
        stream.sender.require_auth();

        let now = env.ledger().timestamp();
        let (vested_now, refund) = Self::quote_cancel(&stream, now)?;
        Self::settle_cancel(&env, stream_id, &mut stream, now, vested_now, refund)?;
        Ok(())
    }

    /// Cancel several streams at once, refunding each unvested remainder to the
    /// sender.
    ///
    /// All streams must share the same `sender`, who authorizes once for the
    /// whole batch — the mirror image of [`batch_withdraw`](Self::batch_withdraw)
    /// and its single `recipient`. Streams need not share a token: each refund
    /// uses its own stream's token, and the returned total is the sum across
    /// them, in each token's own smallest unit.
    ///
    /// Returns a [`BatchCancelOutcome`]: the total refunded on a settled batch,
    /// or the position of the stream that refused it.
    ///
    /// **Atomicity: the batch is all-or-nothing.** Either every stream in the
    /// vector is cancelled, or none is. The whole batch is resolved, checked and
    /// priced before the first storage write, so a batch that fails or refuses
    /// collapses no schedule, moves no token and emits no `cancelled` event;
    /// and if a refund transfer is rejected part-way through the commit phase,
    /// the host discards the writes and transfers already applied alongside it.
    /// Either way the caller is left free to retry with a corrected id list.
    ///
    /// **One instant.** Every refund is priced against a single ledger timestamp
    /// read once, at the top of the call, and each stream is priced exactly once
    /// by [`quote_cancel`](Self::quote_cancel). A batch of streams with
    /// different schedules, cliff positions and pause histories therefore settles
    /// against one consistent "now" rather than drifting across the vector, and
    /// the figure a caller sees for one stream in the batch is the same one that
    /// stream's own `cancel` would have produced at the same instant.
    ///
    /// # Refusals are reported by index
    ///
    /// A stream that exists and belongs to the caller but cannot be cancelled —
    /// created with `cancellable == false`, or already `Cancelled` or
    /// `Depleted` — does not raise a typed `Error`. It refuses the batch and
    /// names its own position: the returned [`BatchCancelOutcome`] carries
    /// `refused_index`, the zero-based offset of the **first** such element in
    /// `stream_ids`, and `refused_reason`, the [`Error`] discriminant that
    /// explains it ([`Error::NotCancellable`] or [`Error::StreamTerminated`]).
    /// `refunded` is `0` in that case, because nothing was touched.
    ///
    /// The index rides in the return value because a Soroban contract error
    /// crosses the wire as a bare `u32` discriminant — `Error(Contract, #N)` —
    /// with no room for a position, and reporting only the *condition* would
    /// leave a caller holding a 16-id vector with no way to learn which element
    /// to drop. Everything that can reject the batch as a whole — an unknown id,
    /// a foreign stream, a duplicate, a batch that is too large — is still a
    /// plain typed `Error`, exactly as for
    /// [`batch_withdraw`](Self::batch_withdraw).
    ///
    /// Refusals are checked in batch order and reported at the first offending
    /// index, so the outcome is deterministic for a given vector.
    ///
    /// # Errors
    ///
    /// * [`Error::EmptyBatch`] — no ids were supplied.
    /// * [`Error::BatchTooLarge`] — more than [`MAX_BATCH_SIZE`] ids. Chunk
    ///   client-side; the SDK does this automatically.
    /// * [`Error::MalformedStreamId`] — a serialized vector element is not a
    ///   `u64`.
    /// * [`Error::DuplicateStreamId`] — the same id appears twice, which would
    ///   otherwise operate on a stale copy of the stream the second time.
    /// * [`Error::StreamNotFound`] — one of the ids does not exist. The whole
    ///   batch fails, matching [`batch_withdraw`](Self::batch_withdraw), which
    ///   also does not skip unknown ids.
    /// * [`Error::Unauthorized`] — one of the streams has a different sender.
    /// * [`Error::Overflow`] — the refunds do not sum to an `i128`.
    /// * [`Error::TokenTransferFailed`] — a refund transfer was rejected by the
    ///   token contract; the whole batch reverts with it.
    /// * [`Error::TokenMissing`] — a stream's token contract is not registered.
    pub fn batch_cancel(
        env: Env,
        sender: Address,
        stream_ids: Vec<u64>,
    ) -> Result<BatchCancelOutcome, Error> {
        let stream_ids = Self::validate_batch_ids(&env, &stream_ids)?;
        Self::reject_duplicate_ids(&stream_ids)?;
        sender.require_auth();

        let now = env.ledger().timestamp();
        let count = stream_ids.len();

        // Resolve and validate the entire batch before changing storage or
        // calling any token contract: existence and ownership first, then a
        // price per stream.
        let mut streams = Vec::new(&env);
        for i in 0..count {
            let stream = storage::peek_stream(&env, stream_ids.get_unchecked(i))?;
            if stream.sender != sender {
                return Err(Error::Unauthorized);
            }
            streams.push_back(stream);
        }

        // Price every member at the same instant. The first member that cannot
        // be cancelled refuses the batch by naming its own index; nothing has
        // been written yet, so refusing costs the caller nothing but the read
        // and leaves every stream — and its TTL — exactly as it was.
        let mut vested = Vec::new(&env);
        let mut refunds = Vec::new(&env);
        let mut total: i128 = 0;
        for i in 0..count {
            let stream = streams.get_unchecked(i);
            let quoted = match Self::quote_cancel(&stream, now) {
                Ok(quoted) => quoted,
                Err(reason) => {
                    return Ok(BatchCancelOutcome {
                        refunded: 0,
                        refused_index: Some(i),
                        refused_reason: Some(reason as u32),
                    })
                }
            };
            total = total.checked_add(quoted.1).ok_or(Error::Overflow)?;
            vested.push_back(quoted.0);
            refunds.push_back(quoted.1);
        }

        // Settle with the figures priced above, not with a second pass over the
        // clock: the same (vested, refund) pair that was checked above is the
        // one written and published.
        for i in 0..count {
            let stream_id = stream_ids.get_unchecked(i);
            let mut stream = streams.get_unchecked(i);
            Self::settle_cancel(
                &env,
                stream_id,
                &mut stream,
                now,
                vested.get_unchecked(i),
                refunds.get_unchecked(i),
            )?;
        }

        Ok(BatchCancelOutcome {
            refunded: total,
            refused_index: None,
            refused_reason: None,
        })
    }

    /// Check the preconditions and price a cancellation of `stream` at `now`.
    ///
    /// Returns `(vested_now, refund)`: the total vested at that instant —
    /// cumulative and inclusive of `withdrawn` — and the unvested remainder
    /// handed back to the sender.
    ///
    /// Pure: it reads no storage and calls no token contract. That is what lets
    /// [`batch_cancel`](Self::batch_cancel) price every member of a batch at one
    /// shared instant, and decide whether to commit at all, before anything is
    /// written. It is the read half of [`settle_cancel`](Self::settle_cancel),
    /// which is the only place that mutates a stream on this path, so a
    /// single-stream cancel and the same stream inside a batch cannot drift
    /// apart.
    ///
    /// # Errors
    ///
    /// * [`Error::NotCancellable`] — created with `cancellable == false`.
    /// * [`Error::StreamTerminated`] — already cancelled or depleted. Checked
    ///   after the flag, so a non-cancellable stream that is also terminal
    ///   reports `NotCancellable`, matching [`cancel`](Self::cancel).
    /// * [`Error::Overflow`] — the accrual arithmetic does not fit in `i128`.
    fn quote_cancel(stream: &Stream, now: u64) -> Result<(i128, i128), Error> {
        if !stream.cancellable {
            return Err(Error::NotCancellable);
        }
        if stream.status.is_terminal() {
            return Err(Error::StreamTerminated);
        }

        let vested_now = accrual::vested(stream, now)?;
        let refund = accrual::refundable(stream, now)?;
        Ok((vested_now, refund))
    }

    /// Collapse `stream` onto `now` and pay `refund` back to its sender.
    ///
    /// The write half of [`quote_cancel`](Self::quote_cancel), taking the
    /// figures that helper already priced at that instant. Both
    /// [`cancel`](Self::cancel) and [`batch_cancel`](Self::batch_cancel) go
    /// through here, so the schedule rewrite, the refund transfer and the
    /// `cancelled` event are identical whether a stream is cancelled alone or
    /// as one member of a batch. `now` is threaded through rather than read from
    /// the ledger so a batch cannot settle member *n* at a later instant than
    /// member *n - 1*.
    ///
    /// A zero refund (nothing unvested — a stream cancelled at or after its
    /// end) issues no transfer at all, per the zero-value policy in
    /// `docs/ABI.md`.
    ///
    /// # Errors
    ///
    /// * [`Error::TokenTransferFailed`] — the refund transfer was rejected.
    /// * [`Error::TokenMissing`] — the stream's token contract is not registered.
    fn settle_cancel(
        env: &Env,
        stream_id: u64,
        stream: &mut Stream,
        now: u64,
        vested_now: i128,
        refund: i128,
    ) -> Result<(), Error> {
        // Issue #1584 — the accounting the `Cancelled` event publishes.
        //
        // `vested_now` is the *total* vested at this instant, cumulative and
        // inclusive of `withdrawn`; `refund` is the unvested remainder handed
        // back to the sender. Conservation (invariant I4) says the two must
        // partition the pre-cancel deposit exactly, with nothing created or
        // destroyed in between. Checked here rather than trusted, because this
        // is the identity every downstream ledger reconciles against, and the
        // test suite now runs it for every member of a batch too.
        //
        // `debug_assert` compiles out of the release profile
        // (`debug-assertions = false`), so this costs the deployed contract
        // nothing while the test suite runs it on every cancellation.
        debug_assert_eq!(
            refund + vested_now,
            stream.deposited,
            "cancel: conservation broken — refund + vested != deposited",
        );
        debug_assert!(
            vested_now >= stream.withdrawn,
            "cancel: vested below what was already withdrawn",
        );

        // Collapse the schedule onto the current point of the stream clock.
        // Clamped at `start_time` so a cancel before the stream opens leaves a
        // zero-length (not negative-length) schedule.
        let settle_at = accrual::stream_time(stream, now).max(stream.start_time);

        stream.deposited = vested_now;
        stream.end_time = settle_at;
        stream.paused_at = None;
        stream.status = StreamStatus::Cancelled;

        let token = stream.token.clone();
        let sender = stream.sender.clone();
        storage::save_stream(env, stream_id, stream);

        if refund > 0 {
            storage::debit_pool(env, &token, refund)?;
            token_transfer(
                env,
                &token,
                &env.current_contract_address(),
                MuxedAddress::from(sender),
                &refund,
            )?;
        }
        // Reconcile the pool now the refund has left it. Checked even when the
        // refund was zero: a rebase since the last operation on this token is
        // exactly as dangerous with nothing to refund.
        verify_pool_balance(env, &token)?;

        // Issue #1584: the event's `vested` is read off the settled stream by
        // the helper (`stream.deposited`, set above), so it cannot disagree with
        // storage. Asserted here so the intent survives a future edit.
        debug_assert_eq!(stream.deposited, vested_now);
        events::cancelled(env, stream_id, stream, refund);
        Ok(())
    }

    /// Recover sender-owned rounding dust after the recipient's claim is settled.
    ///
    /// The sender owns any integer-token residue left by rounding. Recovery is
    /// available after a stream becomes terminal and transfers only the
    /// refundable portion of its outstanding liability. Any amount still
    /// withdrawable by the recipient remains reserved for them.
    pub fn reclaim_dust(env: Env, stream_id: u64) -> Result<i128, Error> {
        let mut stream = storage::load_stream(&env, stream_id)?;
        stream.sender.require_auth();
        if !stream.status.is_terminal() {
            return Ok(0);
        }

        let now = env.ledger().timestamp();
        let liability = accrual::liability(&stream)?;
        let recipient_claim = accrual::withdrawable(&stream, now)?;
        let amount = liability
            .checked_sub(recipient_claim)
            .ok_or(Error::Overflow)?;
        if amount > 0 {
            stream.deposited = stream
                .deposited
                .checked_sub(amount)
                .ok_or(Error::Overflow)?;
            storage::save_stream(&env, stream_id, &stream);
            token_transfer(
                &env,
                &stream.token,
                &env.current_contract_address(),
                MuxedAddress::from(stream.sender.clone()),
                &amount,
            )?;
        }
        Ok(amount)
    }

    /// Pause accrual. Only the sender, and only if `pausable`.
    ///
    /// Pausing freezes the stream's clock and pushes the effective end date
    /// forward by the paused duration. Total value delivered stays constant; the
    /// schedule simply stretches. The recipient can still withdraw what they
    /// already earned.
    pub fn pause(env: Env, stream_id: u64) -> Result<(), Error> {
        // Emergency halt (#1818): refuse state changes before anything else.
        Self::require_not_halted(&env)?;
        let mut stream = storage::load_stream(&env, stream_id)?;
        stream.sender.require_auth();

        if !stream.pausable {
            return Err(Error::NotPausable);
        }
        if stream.status.is_terminal() {
            return Err(Error::StreamTerminated);
        }
        if stream.status == StreamStatus::Paused {
            return Err(Error::StreamAlreadyPaused);
        }

        let now = env.ledger().timestamp();
        let old_vested = accrual::vested(&stream, now)?;

        stream.paused_at = Some(now);
        stream.status = StreamStatus::Paused;

        if accrual::vested(&stream, now)? < old_vested {
            return Err(Error::VestedDecreased);
        }

        storage::save_stream(&env, stream_id, &stream);

        events::paused(&env, stream_id, &stream, now);
        Ok(())
    }

    /// Resume a paused stream, absorbing the paused interval into
    /// `paused_total` so the clock picks up exactly where it stopped.
    pub fn resume(env: Env, stream_id: u64) -> Result<(), Error> {
        // Emergency halt (#1818): refuse state changes before anything else.
        Self::require_not_halted(&env)?;
        let mut stream = storage::load_stream(&env, stream_id)?;
        stream.sender.require_auth();

        if stream.status.is_terminal() {
            return Err(Error::StreamTerminated);
        }
        let paused_at = match stream.paused_at {
            Some(t) => t,
            None => return Err(Error::StreamNotPaused),
        };
        if stream.status != StreamStatus::Paused {
            return Err(Error::StreamNotPaused);
        }

        let now = env.ledger().timestamp();
        let old_vested = accrual::vested(&stream, now)?;

        let paused_duration = now.saturating_sub(paused_at);
        stream.paused_total = stream
            .paused_total
            .checked_add(paused_duration)
            .ok_or(Error::Overflow)?;
        stream.paused_at = None;
        stream.status = StreamStatus::Active;

        if accrual::vested(&stream, now)? < old_vested {
            return Err(Error::VestedDecreased);
        }

        storage::save_stream(&env, stream_id, &stream);

        events::resumed(&env, stream_id, &stream, paused_duration);
        Ok(())
    }

    /// Reassign a stream's future payouts to a new recipient. Sender auth.
    ///
    /// Available only if the stream was created with `transferable == true`.
    /// A compliance-bound sender — payroll, a KYC'd grant program — can pin the
    /// payee at creation by passing `false`.
    ///
    /// Any balance the old recipient had already accrued but not withdrawn moves
    /// with the stream. Recipients should withdraw before transferring.
    ///
    /// Transfer is allowed at the cliff and end timestamps, and the new
    /// recipient receives any claim that is withdrawable at those boundaries.
    /// A cancelled stream may still be transferred while it has an unwithdrawn
    /// tail; a depleted stream cannot be transferred.
    ///
    /// # Delegate grants
    ///
    /// A transfer changes only the `recipient` field; it does **not** touch
    /// delegate grants. Grants are scoped to the stream, not to the recipient
    /// who issued them, so every grant live before the transfer is still live
    /// after it — including recipient-issued `WITHDRAW` and
    /// `TRANSFER_RECIPIENT` grants, which now belong to whoever holds the
    /// recipient slot. That makes revocation follow the slot rather than the
    /// person: the new recipient can revoke a surviving grant immediately, the
    /// old recipient can neither grant nor revoke anything further, and
    /// sender-issued grants (`CANCEL`, `PAUSE`, `RESUME`, `TOP_UP`) are
    /// untouched because the sender did not change.
    ///
    /// The rule is stated in `docs/delegation-revocation.md` and asserted for
    /// every permission bit by `test::delegation`.
    pub fn transfer_recipient(
        env: Env,
        stream_id: u64,
        new_recipient: Address,
    ) -> Result<(), Error> {
        // Emergency halt (#1818): refuse state changes before anything else.
        Self::require_not_halted(&env)?;
        let mut stream = storage::load_stream(&env, stream_id)?;
        // #1637 hardens recipient-transfer authorization to the sender: the
        // party who funded the stream keeps control over who is paid out.
        // Granting this to the current recipient is the *delegate* path
        // (`delegate_transfer_recipient`), gated on a recipient-issued grant.
        stream.sender.require_auth();

        if !stream.transferable {
            return Err(Error::NotTransferable);
        }
        // A stream with no claim left is not reassignable. This covers both
        // `Depleted` streams and cancelled streams whose tail has been fully
        // drawn (where `Cancelled` is intentionally sticky, so checking only
        // the status would miss it).
        if stream.status == StreamStatus::Depleted || stream.withdrawn >= stream.deposited {
            return Err(Error::StreamTerminated);
        }
        if new_recipient == stream.sender {
            return Err(Error::SelfStream);
        }

        let old_recipient = stream.recipient.clone();
        if old_recipient == new_recipient {
            return Err(Error::RepeatedTransfer);
        }

        let now = env.ledger().timestamp();
        let old_vested = accrual::vested(&stream, now)?;

        stream.recipient = new_recipient.clone();

        if accrual::vested(&stream, now)? < old_vested {
            return Err(Error::VestedDecreased);
        }

        storage::save_stream(&env, stream_id, &stream);

        events::recipient_transferred(
            &env,
            stream_id,
            &stream.sender,
            &old_recipient,
            &new_recipient,
        );
        Ok(())
    }

    // ---------------------------------------------------------------------
    // Delegation
    // ---------------------------------------------------------------------

    /// Grant a delegate permission to call specific operations on a stream.
    ///
    /// `ops` is a bitmask of [`Op`] constants. `expires_at` is an optional
    /// Unix timestamp after which the grant is no longer valid; pass `None`
    /// for a grant that does not expire on its own. Only the authoritative
    /// party for the requested ops may call this:
    ///
    /// * Sender-side ops (`CANCEL`, `PAUSE`, `RESUME`, `TOP_UP`): sender auth.
    /// * Recipient-side ops (`WITHDRAW`, `TRANSFER_RECIPIENT`): recipient auth.
    /// * Mixed grants must come from both parties; callers should split them.
    ///
    /// Granting over an existing grant replaces it entirely.
    ///
    /// # Errors
    ///
    /// * [`Error::StreamNotFound`] — no stream with this id.
    /// * [`Error::StreamTerminated`] — stream is cancelled or depleted.
    /// * [`Error::Unauthorized`] — caller is not the required party for any of
    ///   the requested ops.
    pub fn grant_delegate(
        env: Env,
        stream_id: u64,
        grantor: Address,
        delegate: Address,
        ops: u32,
        expires_at: Option<u64>,
    ) -> Result<(), Error> {
        // Emergency halt (#1818): refuse state changes before anything else.
        Self::require_not_halted(&env)?;
        let stream = storage::load_stream(&env, stream_id)?;
        if stream.status.is_terminal() {
            return Err(Error::StreamTerminated);
        }

        let sender_ops = op::CANCEL | op::PAUSE | op::RESUME | op::TOP_UP;
        let recipient_ops = op::WITHDRAW | op::TRANSFER_RECIPIENT;

        // The grantor must be the party that owns the ops being delegated.
        // Mixed calls are rejected; split into two grants instead.
        let needs_sender = ops & sender_ops != 0;
        let needs_recipient = ops & recipient_ops != 0;

        if needs_sender && needs_recipient {
            return Err(Error::Unauthorized);
        }
        if needs_sender {
            if grantor != stream.sender {
                return Err(Error::Unauthorized);
            }
            grantor.require_auth();
        } else if needs_recipient {
            if grantor != stream.recipient {
                return Err(Error::Unauthorized);
            }
            grantor.require_auth();
        } else {
            // ops == 0 is a no-op; treat it as success.
            return Ok(());
        }

        let grant = DelegateGrant { ops, expires_at };
        storage::save_delegate(&env, stream_id, &delegate, &grant);

        events::delegate_granted(&env, stream_id, &grantor, &delegate, ops, expires_at);
        Ok(())
    }

    /// Revoke a previously-issued delegate grant.
    ///
    /// # Authority follows the party, not the person
    ///
    /// The sender / recipient test below reads the stream's *current* parties,
    /// so after a [`transfer_recipient`](Self::transfer_recipient) the new
    /// recipient holds the recipient half of the check: they can revoke a grant
    /// the old recipient issued, and the old recipient — no longer a party to
    /// the stream — is rejected with [`Error::Unauthorized`]. Grants themselves
    /// survive the transfer; see `docs/delegation-revocation.md`.
    ///
    /// Takes effect immediately — the delegate's next call will be rejected.
    /// Funds the delegate has already moved (e.g. via a prior `withdraw`) are
    /// unaffected: revocation only stops future invocations.
    ///
    /// # Same-ledger ordering
    ///
    /// Revocation is **ordered, not retroactive**. This call removes the grant
    /// from storage; every invocation ordered after it — in the same ledger or
    /// any later one — reads no grant and fails with
    /// [`Error::DelegateNotPermitted`]. An invocation ordered *before* the
    /// revocation is honoured and is not unwound: already-completed calls are
    /// unaffected, only the ability to make new ones is withdrawn.
    ///
    /// There is no grace period and no distinction between same-ledger and
    /// cross-ledger calls — a single storage write decides both. Ordering
    /// *within* a ledger is the network's transaction application order, not
    /// something this contract selects; the guarantee is only that whichever
    /// order the network applies, the delegate call on the later side of the
    /// revocation is rejected. `test::delegation` pins both orders for every
    /// permission bit; `docs/delegation-revocation.md` states the guarantee.
    ///
    /// Silently succeeds if no grant exists (idempotent).
    ///
    /// The grantor must be either the sender or the recipient of the stream.
    ///
    /// # Errors
    ///
    /// * [`Error::StreamNotFound`] — no stream with this id.
    /// * [`Error::Unauthorized`] — caller is neither sender nor recipient.
    pub fn revoke_delegate(
        env: Env,
        stream_id: u64,
        grantor: Address,
        delegate: Address,
    ) -> Result<(), Error> {
        // Emergency halt (#1818): refuse state changes before anything else.
        Self::require_not_halted(&env)?;
        let stream = storage::load_stream(&env, stream_id)?;

        if grantor != stream.sender && grantor != stream.recipient {
            return Err(Error::Unauthorized);
        }
        grantor.require_auth();

        storage::remove_delegate(&env, stream_id, &delegate);
        events::delegate_revoked(&env, stream_id, &grantor, &delegate);
        Ok(())
    }

    /// Withdraw as a delegate. The `delegate` address must hold a valid
    /// grant with [`op::WITHDRAW`] for this stream.
    pub fn delegate_withdraw(
        env: Env,
        stream_id: u64,
        delegate: Address,
        amount: Option<i128>,
    ) -> Result<i128, Error> {
        // Emergency halt (#1818): refuse state changes before anything else.
        Self::require_not_halted(&env)?;
        Self::check_delegate(&env, stream_id, &delegate, op::WITHDRAW)?;
        let mut stream = storage::load_stream(&env, stream_id)?;

        let now = env.ledger().timestamp();
        let available = accrual::withdrawable(&stream, now)?;
        if available == 0 {
            if stream.status.is_terminal() {
                return Err(Error::StreamTerminated);
            }
            return Err(Error::NothingToWithdraw);
        }

        let payout = match amount {
            None => available,
            Some(requested) => {
                if requested <= 0 {
                    return Err(Error::InvalidAmount);
                }
                if requested > available {
                    return Err(Error::InsufficientWithdrawable);
                }
                requested
            }
        };

        Self::apply_withdrawal(&env, stream_id, &mut stream, payout)?;
        // Reconcile the pool against the token's real balance now the payout
        // has left it. A rebase between this token's last operation and this
        // one shows up as a shortfall and rolls the withdrawal back
        // (`Error::PoolBalanceDrift`) instead of letting the pool drift further.
        verify_pool_balance(&env, &stream.token)?;
        Ok(payout)
    }

    /// Cancel as a delegate. Requires [`op::CANCEL`] grant.
    pub fn delegate_cancel(env: Env, stream_id: u64, delegate: Address) -> Result<(), Error> {
        // Emergency halt (#1818): refuse state changes before anything else.
        Self::require_not_halted(&env)?;
        Self::check_delegate(&env, stream_id, &delegate, op::CANCEL)?;
        let mut stream = storage::load_stream(&env, stream_id)?;

        if !stream.cancellable {
            return Err(Error::NotCancellable);
        }
        if stream.status.is_terminal() {
            return Err(Error::StreamTerminated);
        }

        let now = env.ledger().timestamp();
        let vested_now = accrual::vested(&stream, now)?;
        let refund = accrual::refundable(&stream, now)?;
        let settle_at = accrual::stream_time(&stream, now).max(stream.start_time);

        stream.deposited = vested_now;
        stream.end_time = settle_at;
        stream.paused_at = None;
        stream.status = StreamStatus::Cancelled;

        let token = stream.token.clone();
        let sender = stream.sender.clone();
        storage::save_stream(&env, stream_id, &stream);

        if refund > 0 {
            storage::debit_pool(&env, &token, refund)?;
            token_transfer(
                &env,
                &token,
                &env.current_contract_address(),
                MuxedAddress::from(sender),
                &refund,
            )?;
        }
        // Reconcile the pool now the refund has left it (see `cancel`).
        verify_pool_balance(&env, &token)?;

        // `vested` is derived from the post-cancel `stream.deposited` inside
        // the event, so it is not passed separately.
        events::cancelled(&env, stream_id, &stream, refund);
        Ok(())
    }

    /// Pause as a delegate. Requires [`op::PAUSE`] grant.
    pub fn delegate_pause(env: Env, stream_id: u64, delegate: Address) -> Result<(), Error> {
        // Emergency halt (#1818): refuse state changes before anything else.
        Self::require_not_halted(&env)?;
        Self::check_delegate(&env, stream_id, &delegate, op::PAUSE)?;
        let mut stream = storage::load_stream(&env, stream_id)?;

        if !stream.pausable {
            return Err(Error::NotPausable);
        }
        if stream.status.is_terminal() {
            return Err(Error::StreamTerminated);
        }
        if stream.status == StreamStatus::Paused {
            return Err(Error::StreamAlreadyPaused);
        }

        let now = env.ledger().timestamp();
        stream.paused_at = Some(now);
        stream.status = StreamStatus::Paused;
        storage::save_stream(&env, stream_id, &stream);

        events::paused(&env, stream_id, &stream, now);
        Ok(())
    }

    /// Resume as a delegate. Requires [`op::RESUME`] grant.
    pub fn delegate_resume(env: Env, stream_id: u64, delegate: Address) -> Result<(), Error> {
        // Emergency halt (#1818): refuse state changes before anything else.
        Self::require_not_halted(&env)?;
        Self::check_delegate(&env, stream_id, &delegate, op::RESUME)?;
        let mut stream = storage::load_stream(&env, stream_id)?;

        if stream.status.is_terminal() {
            return Err(Error::StreamTerminated);
        }
        let paused_at = match stream.paused_at {
            Some(t) => t,
            None => return Err(Error::StreamNotPaused),
        };
        if stream.status != StreamStatus::Paused {
            return Err(Error::StreamNotPaused);
        }

        let now = env.ledger().timestamp();
        let paused_duration = now.saturating_sub(paused_at);
        stream.paused_total = stream
            .paused_total
            .checked_add(paused_duration)
            .ok_or(Error::Overflow)?;
        stream.paused_at = None;
        stream.status = StreamStatus::Active;
        storage::save_stream(&env, stream_id, &stream);

        events::resumed(&env, stream_id, &stream, paused_duration);
        Ok(())
    }

    /// Top up as a delegate. Requires [`op::TOP_UP`] grant.
    pub fn delegate_top_up(
        env: Env,
        stream_id: u64,
        delegate: Address,
        amount: i128,
    ) -> Result<(), Error> {
        // Emergency halt (#1818): refuse state changes before anything else.
        Self::require_not_halted(&env)?;
        Self::check_delegate(&env, stream_id, &delegate, op::TOP_UP)?;
        let mut stream = storage::load_stream(&env, stream_id)?;

        if stream.status.is_terminal() {
            return Err(Error::StreamTerminated);
        }
        if amount <= 0 {
            return Err(Error::InvalidAmount);
        }

        let now = env.ledger().timestamp();
        if accrual::stream_time(&stream, now) >= stream.end_time {
            return Err(Error::StreamMatured);
        }

        let current_duration = accrual::duration(&stream) as i128;
        let scaled = amount
            .checked_mul(current_duration)
            .ok_or(Error::Overflow)?;
        let delta = scaled
            .checked_div(stream.deposited)
            .ok_or(Error::Overflow)?;
        if delta < 0 || delta > u64::MAX as i128 {
            return Err(Error::Overflow);
        }
        if delta == 0 {
            return Err(Error::TopUpTooSmall);
        }

        let new_deposited = stream
            .deposited
            .checked_add(amount)
            .ok_or(Error::Overflow)?;
        let new_end = stream
            .end_time
            .checked_add(delta as u64)
            .ok_or(Error::Overflow)?;
        let new_duration = new_end
            .checked_sub(stream.start_time)
            .ok_or(Error::Overflow)?;
        if new_duration > MAX_STREAM_DURATION {
            return Err(Error::DurationTooLong);
        }

        new_deposited
            .checked_mul(new_duration as i128)
            .ok_or(Error::Overflow)?;
        if new_deposited < new_duration as i128 {
            return Err(Error::DepositRateTooLow);
        }

        let token = stream.token.clone();
        let sender = stream.sender.clone();
        stream.deposited = new_deposited;
        stream.end_time = new_end;
        storage::save_stream(&env, stream_id, &stream);

        // Tokens come from the sender — require their auth even though a
        // delegate triggered this call.
        sender.require_auth();
        pull_deposit(&env, &token, &sender, &amount)?;
        // Same reconciliation as `top_up`: a rebase on this token is caught
        // here and rolls the delegated top-up back.
        verify_pool_balance(&env, &token)?;

        events::topped_up(&env, stream_id, &stream, amount);
        Ok(())
    }

    /// Transfer recipient as a delegate. Requires [`op::TRANSFER_RECIPIENT`] grant.
    ///
    /// This is the authorisation-gated twin of
    /// [`transfer_recipient`](Self::transfer_recipient). Once the grant is
    /// verified it repeats the direct path's four stream-level guards —
    /// `NotTransferable`, `StreamTerminated`, `SelfStream` and
    /// `RepeatedTransfer`, in that order — so the two entry points reject the
    /// same stream states with the same discriminants and a client does not
    /// have to special-case which one it called.
    ///
    /// A reassignment to the *current* recipient is rejected with
    /// [`Error::RepeatedTransfer`] rather than silently accepted. The error is
    /// the caller's signal that the stream was not re-pointed, and it is what
    /// makes a replay (or a stale `new_recipient`) distinguishable from a
    /// first transfer that landed — the same replay guarantee the direct path
    /// has carried since #1637.
    ///
    /// Nothing is cleared by the transfer, so the grant that authorised this
    /// call — and every other grant on the stream — survives it. A delegate
    /// acting for the old recipient therefore keeps its rights until the new
    /// recipient revokes them.
    pub fn delegate_transfer_recipient(
        env: Env,
        stream_id: u64,
        delegate: Address,
        new_recipient: Address,
    ) -> Result<(), Error> {
        // Emergency halt (#1818): refuse state changes before anything else.
        Self::require_not_halted(&env)?;
        Self::check_delegate(&env, stream_id, &delegate, op::TRANSFER_RECIPIENT)?;
        let mut stream = storage::load_stream(&env, stream_id)?;

        if !stream.transferable {
            return Err(Error::NotTransferable);
        }
        // Same settled-claim rule as `transfer_recipient`: a stream with no
        // unwithdrawn claim is not reassignable.
        if stream.status == StreamStatus::Depleted || stream.withdrawn >= stream.deposited {
            return Err(Error::StreamTerminated);
        }
        if new_recipient == stream.sender {
            return Err(Error::SelfStream);
        }

        let old_recipient = stream.recipient.clone();
        if old_recipient == new_recipient {
            // Parity with `transfer_recipient` (#1637): a no-op reassignment is
            // an error, not a silent success, so a caller can tell a replay or
            // a stale `new_recipient` from a transfer that actually moved the
            // stream. Before #1827 this path returned `Ok(())`, which made the
            // delegated variant the one place a repeated transfer did not
            // surface `RepeatedTransfer`.
            return Err(Error::RepeatedTransfer);
        }

        stream.recipient = new_recipient.clone();
        storage::save_stream(&env, stream_id, &stream);

        events::recipient_transferred(
            &env,
            stream_id,
            &stream.sender,
            &old_recipient,
            &new_recipient,
        );
        Ok(())
    }

    // ---------------------------------------------------------------------
    // Views
    // ---------------------------------------------------------------------

    /// Full stream state.
    ///
    /// Views deliberately do **not** extend the entry's TTL. They are called
    /// through simulation by the SDK and UI, where a write to the footprint is
    /// at best noise and at worst confusing. Keeping a stream alive is the
    /// explicit job of [`extend_stream_ttl`](Self::extend_stream_ttl).
    ///
    /// Returns [`Error::StreamNotFound`] when the id is missing or deleted.
    pub fn get_stream(env: Env, stream_id: u64) -> Result<Stream, Error> {
        storage::peek_stream(&env, stream_id)
    }

    /// Amount the recipient could withdraw right now.
    ///
    /// Returns [`Error::StreamNotFound`] when the id is missing or deleted;
    /// zero means the stream exists but has no currently withdrawable funds.
    pub fn withdrawable_of(env: Env, stream_id: u64) -> Result<i128, Error> {
        let stream = storage::peek_stream(&env, stream_id)?;
        accrual::withdrawable(&stream, env.ledger().timestamp())
    }

    /// Total earned by the recipient since `start_time`, withdrawn or not.
    ///
    /// Returns [`Error::StreamNotFound`] when the id is missing or deleted.
    pub fn vested_of(env: Env, stream_id: u64) -> Result<i128, Error> {
        let stream = storage::peek_stream(&env, stream_id)?;
        accrual::vested(&stream, env.ledger().timestamp())
    }

    /// Amount that would be refunded to the sender if they cancelled right now.
    ///
    /// Returns [`Error::StreamNotFound`] when the id is missing or deleted.
    pub fn refundable_of(env: Env, stream_id: u64) -> Result<i128, Error> {
        let stream = storage::peek_stream(&env, stream_id)?;
        accrual::refundable(&stream, env.ledger().timestamp())
    }

    /// Number of streams ever created. Ids run `0..stream_count()`.
    pub fn stream_count(env: Env) -> u64 {
        storage::stream_count(&env)
    }

    /// Whether a stream entry is currently readable.
    ///
    /// Returns `false` both for ids that were never issued and for entries that
    /// have been archived. Compare against [`stream_count`](Self::stream_count)
    /// to tell those apart: an id below the count that does not exist has been
    /// archived and needs restoring.
    pub fn stream_exists(env: Env, stream_id: u64) -> bool {
        storage::stream_exists(&env, stream_id)
    }

    // ---------------------------------------------------------------------
    // Maintenance
    // ---------------------------------------------------------------------

    /// Extend a stream entry's TTL. **Permissionless** — anyone may pay.
    ///
    /// Returns the number of ledgers the entry is now good for.
    ///
    /// This is the keeper hook. It is unauthenticated on purpose: a recipient's
    /// claim must never depend on the sender's continued goodwill, and a
    /// third-party keeper sweeping streams that approach expiry should not need
    /// anyone's permission to do so. There is nothing to grief here — the caller
    /// only ever *pays* rent, and TTL extension cannot move funds or change
    /// stream state.
    ///
    /// Multi-year streams need this periodically no matter how generously the
    /// contract extends at creation, because no entry may exceed the network's
    /// `max_entry_ttl`.
    ///
    /// # Terminal streams
    ///
    /// **Extending the TTL of a `Cancelled` or `Depleted` stream is rejected
    /// with [`Error::StreamTerminated`].** Terminal streams have settled all
    /// accounting and no future state change is possible. Their entries decay
    /// from the floor set at cancellation/depletion to zero under normal Soroban
    /// rent rules; the caller should not pay indefinitely for a record that will
    /// never change. Keeping terminal records accessible is only necessary while
    /// a recipient still has an unwithdrawn tail (which is the `Cancelled` but
    /// not-yet-drained case) — that window is covered by the floor TTL the
    /// contract applies at the time of cancellation/depletion. If the entry has
    /// since archived, it must be restored via a `RestoreFootprint` operation
    /// rather than extended.
    ///
    /// # Errors
    ///
    /// * [`Error::StreamNotFound`] — no stream with this id.
    /// * [`Error::StreamTerminated`] — stream is `Cancelled` or `Depleted`.
    pub fn extend_stream_ttl(env: Env, stream_id: u64) -> Result<u32, Error> {
        // Emergency halt (#1818): refuse state changes before anything else.
        Self::require_not_halted(&env)?;
        // Authorization: permissionless by design — see doc comment. Any caller
        // may pay rent for any stream. The caller has no address parameter and
        // no `require_auth` is invoked.
        let stream = storage::peek_stream(&env, stream_id)?;

        // Terminal streams (Cancelled, Depleted) have settled all accounting.
        // Reject the extension so callers are not silently charged indefinite
        // rent for a record that can no longer change. The floor TTL applied at
        // the time of cancellation/depletion covers the withdrawal tail; after
        // that the entry may archive and must be restored via RestoreFootprint.
        if stream.status.is_terminal() {
            return Err(Error::StreamTerminated);
        }

        let target = storage::ttl_target_ledgers(&env, &stream);
        storage::extend_stream(&env, stream_id, &stream);
        storage::extend_instance(&env);

        events::ttl_extended(&env, stream_id, target);
        Ok(target)
    }

    /// Extend several streams' TTLs in one transaction. Permissionless.
    ///
    /// Same [`MAX_BATCH_SIZE`] cap as [`batch_withdraw`](Self::batch_withdraw).
    ///
    /// **The sweep is per-item, not atomic.** Unknown ids are skipped rather
    /// than failing the batch, so a keeper working from a slightly stale index
    /// does not lose the whole sweep to one bad id. Unlike `batch_withdraw`,
    /// duplicate ids are rejected up-front: passing the same id twice returns
    /// [`Error::DuplicateStreamId`] rather than attempting to extend it twice.
    /// Empty, oversized, and malformed vectors are rejected before the sweep
    /// starts. Returns how many entries were actually extended.
    ///
    /// # Terminal streams
    ///
    /// **Any `Cancelled` or `Depleted` stream in the batch causes the entire
    /// call to fail with [`Error::StreamTerminated`].** This mirrors the single
    /// entry-point policy in [`extend_stream_ttl`](Self::extend_stream_ttl) and
    /// ensures both paths enforce the same rule: callers may not extend the TTL
    /// of a settled stream. A keeper sweep should filter out terminal stream ids
    /// before submitting a batch; the indexer's `status` field is the signal.
    ///
    /// Unknown ids continue to be skipped — a stale index entry for a stream
    /// that does not exist (or has archived) does not abort the sweep.
    ///
    /// # Errors
    ///
    /// * [`Error::EmptyBatch`] — no ids provided.
    /// * [`Error::BatchTooLarge`] — more than [`MAX_BATCH_SIZE`] ids.
    /// * [`Error::MalformedStreamId`] — an element is not a valid `u64`.
    /// * [`Error::DuplicateStreamId`] — the same id appears more than once.
    /// * [`Error::StreamTerminated`] — at least one id is `Cancelled` or `Depleted`.
    pub fn batch_extend_ttl(env: Env, stream_ids: Vec<u64>) -> Result<u32, Error> {
        // Emergency halt (#1818): refuse state changes before anything else.
        Self::require_not_halted(&env)?;
        // Authorization: permissionless — same policy as `extend_stream_ttl`.
        let stream_ids = Self::validate_batch_ids(&env, &stream_ids)?;
        Self::reject_duplicate_ids(&stream_ids)?;

        let mut extended = 0u32;
        for stream_id in stream_ids.iter() {
            if let Ok(stream) = storage::peek_stream(&env, stream_id) {
                // Reject the entire batch if any stream is terminal. This
                // matches the single-stream policy and prevents a caller from
                // inadvertently paying rent for settled records. Unknown ids
                // are still skipped (keeper resilience), but a known-terminal
                // id is an explicit error.
                if stream.status.is_terminal() {
                    return Err(Error::StreamTerminated);
                }
                let target = storage::ttl_target_ledgers(&env, &stream);
                storage::extend_stream(&env, stream_id, &stream);
                events::ttl_extended(&env, stream_id, target);
                extended += 1;
            }
        }
        storage::extend_instance(&env);
        Ok(extended)
    }

    // ---------------------------------------------------------------------
    // Emergency halt (issue #1818)
    // ---------------------------------------------------------------------

    /// Install the contract-level halt operator. **One-shot: there is no
    /// rotation.**
    ///
    /// This is the only authorization the stream contract has, and it is
    /// opt-in: a deployment that never calls this has no operator and cannot
    /// be halted at all, which is where every deployment starts. A second call
    /// returns [`Error::HaltOperatorAlreadySet`] rather than replacing the
    /// first, so the operator cannot be swapped under an incident — replacing
    /// one means deploying a new contract.
    ///
    /// `operator` must authorize the call. The operator's only powers are
    /// [`halt`](Self::halt) and [`resume_contract`](Self::resume_contract): it
    /// cannot move funds, cancel a stream, change a schedule, or withdraw. It
    /// also cannot be removed.
    pub fn set_halt_operator(env: Env, operator: Address) -> Result<(), Error> {
        operator.require_auth();
        if storage::halt_operator(&env).is_some() {
            return Err(Error::HaltOperatorAlreadySet);
        }
        storage::set_halt_operator(&env, &operator);
        events::halt_operator_set(&env, &operator);
        Ok(())
    }

    /// Halt the whole contract. Operator only.
    ///
    /// Every state-changing entry point then refuses with
    /// [`Error::ContractHalted`] until [`resume_contract`](Self::resume_contract)
    /// is called. Every read method (`get_stream`, `withdrawable_of`,
    /// `vested_of`, `refundable_of`, `stream_count`, `stream_exists`,
    /// [`halted`](Self::halted), [`halt_operator`](Self::halt_operator)) keeps
    /// answering normally, so integrators can still observe on-chain state
    /// while the incident is handled.
    ///
    /// The halt is a **circuit breaker, not a settlement**: it moves no funds
    /// and rewrites no stream. Accrual is a pure function of ledger time, so a
    /// stream continues to vest while halted and `withdrawable_of` keeps
    /// climbing; what stops is settlement. Lifting the halt resumes from
    /// exactly the state that was halted.
    ///
    /// There is no timeout — the halt ends only when the operator resumes it.
    /// Returns [`Error::HaltOperatorNotSet`] when no operator was ever
    /// installed and [`Error::ContractAlreadyHalted`] when already halted.
    pub fn halt(env: Env) -> Result<(), Error> {
        let operator = storage::halt_operator(&env).ok_or(Error::HaltOperatorNotSet)?;
        operator.require_auth();
        if storage::is_halted(&env) {
            return Err(Error::ContractAlreadyHalted);
        }
        storage::set_halt(&env);
        events::contract_halted(&env, &operator, env.ledger().timestamp());
        Ok(())
    }

    /// Lift the contract-level halt. Operator only.
    ///
    /// Settlement is restored for every stream with the state it had when the
    /// halt was engaged. Emits the matching event. Returns
    /// [`Error::HaltOperatorNotSet`] when no operator was ever installed and
    /// [`Error::ContractNotHalted`] when the contract is not halted.
    pub fn resume_contract(env: Env) -> Result<(), Error> {
        let operator = storage::halt_operator(&env).ok_or(Error::HaltOperatorNotSet)?;
        operator.require_auth();
        let halted_at = storage::halt_started_at(&env).ok_or(Error::ContractNotHalted)?;
        storage::clear_halt(&env);
        let now = env.ledger().timestamp();
        events::contract_resumed(&env, &operator, now, now.saturating_sub(halted_at));
        Ok(())
    }

    /// Whether the contract-level halt is engaged.
    ///
    /// `false` until an operator is installed and calls
    /// [`halt`](Self::halt). Reads are unaffected by the halt, so this view
    /// always answers.
    pub fn halted(env: Env) -> bool {
        storage::is_halted(&env)
    }

    /// The installed halt operator, or `None` when the contract has opted out
    /// of the emergency stop entirely.
    pub fn halt_operator(env: Env) -> Option<Address> {
        storage::halt_operator(&env)
    }

    /// Whether the deployed contract can be replaced in place.
    ///
    /// Always `false` — this is the on-chain statement of the upgrade
    /// posture documented in `docs/ABI.md` and `docs/MIGRATION.md`. It is a
    /// constant, not a storage read, so it answers even before any stream
    /// exists and cannot be changed by the halt operator or anyone else.
    pub fn upgradeable(_env: Env) -> bool {
        UPGRADEABLE
    }

    // ---------------------------------------------------------------------
    // Internal
    // ---------------------------------------------------------------------

    /// Refuse a state-changing call while the contract-level halt is engaged.
    ///
    /// Called first, before authorization and before any other precondition,
    /// in every mutating entry point. That ordering is deliberate: while the
    /// contract is halted the caller learns the contract is stopped (34)
    /// rather than whether their call would otherwise have been valid — and
    /// the contract does no work it is going to throw away.
    ///
    /// The check is one instance-storage lookup, and it is the same lookup in
    /// every entry point, which is what keeps "every mutating entry point is
    /// refused" a property of one function instead of eighteen.
    fn require_not_halted(env: &Env) -> Result<(), Error> {
        if storage::is_halted(env) {
            return Err(Error::ContractHalted);
        }
        Ok(())
    }

    /// Verify that `caller` holds a valid, unexpired delegate grant for `op`
    /// on `stream_id`, then call `caller.require_auth()`.
    ///
    /// Returns `DelegateNotPermitted` if no grant exists or the grant does not
    /// cover `op`. Returns `DelegateExpired` if the grant exists but has passed
    /// its expiry. On success the host will validate the caller's auth.
    fn check_delegate(env: &Env, stream_id: u64, caller: &Address, op: u32) -> Result<(), Error> {
        match storage::load_delegate(env, stream_id, caller) {
            None => Err(Error::DelegateNotPermitted),
            Some(grant) => {
                if let Some(expires) = grant.expires_at {
                    if env.ledger().timestamp() > expires {
                        return Err(Error::DelegateExpired);
                    }
                }
                if grant.ops & op == 0 {
                    return Err(Error::DelegateNotPermitted);
                }
                caller.require_auth();
                Ok(())
            }
        }
    }

    /// Pre-flight shared by the batch entry points: reject empty, oversized,
    /// and malformed vectors, and return a validated copy.
    ///
    /// `MalformedStreamId` covers a serialized element that does not decode as
    /// a `u64`; the batch entry points take `Vec<u64>` straight from the ABI,
    /// and element-level conversion errors would otherwise surface as opaque
    /// host errors instead of typed ones.
    fn validate_batch_ids(env: &Env, stream_ids: &Vec<u64>) -> Result<Vec<u64>, Error> {
        let count = stream_ids.len();
        if count == 0 {
            return Err(Error::EmptyBatch);
        }
        if count > MAX_BATCH_SIZE {
            return Err(Error::BatchTooLarge);
        }

        let mut validated = Vec::new(env);
        for raw_id in stream_ids.to_vals().iter() {
            let stream_id =
                u64::try_from_val(env, &raw_id).map_err(|_| Error::MalformedStreamId)?;
            validated.push_back(stream_id);
        }
        Ok(validated)
    }

    /// Reject a batch that names the same stream twice. Processing the same
    /// stream twice would otherwise operate on a stale copy the second time.
    /// Both `batch_withdraw` and `batch_extend_ttl` reject duplicates.
    fn reject_duplicate_ids(stream_ids: &Vec<u64>) -> Result<(), Error> {
        let count = stream_ids.len();
        for i in 0..count {
            for j in (i + 1)..count {
                if stream_ids.get_unchecked(i) == stream_ids.get_unchecked(j) {
                    return Err(Error::DuplicateStreamId);
                }
            }
        }
        Ok(())
    }

    /// Shared tail of [`withdraw`](Self::withdraw) and
    /// [`batch_withdraw`](Self::batch_withdraw): update accounting, persist,
    /// pay out, emit.
    ///
    /// # Atomicity of bookkeeping vs. token transfer
    ///
    /// State (`stream.withdrawn`, `stream.status`) is written to storage before
    /// the token `transfer` call.  This is the standard
    /// checks-effects-interactions ordering.  Soroban forbids reentrancy
    /// outright, so there is no classical reentrancy risk here.
    ///
    /// The correctness argument for atomicity in the failure case is that
    /// **every host trap propagates as a Rust panic that unwinds the entire
    /// transaction**.  If `token::Client::transfer` panics (e.g. because the
    /// recipient is deauthorized on a Stellar Asset Contract, or because the
    /// token contract itself traps for any reason), the Soroban host discards
    /// every storage write made in this invocation — including the
    /// `save_stream` call — before returning the error to the caller.  No
    /// partial state leaks: `stream.withdrawn` is never permanently incremented
    /// unless the matching tokens actually leave the contract's pool.
    ///
    /// `test::withdrawal_atomicity` proves this with two failure injection
    /// mechanisms: (1) SAC `set_authorized(recipient, false)` and (2) a
    /// custom contract that always panics registered at the token address.
    fn apply_withdrawal(
        env: &Env,
        stream_id: u64,
        stream: &mut Stream,
        payout: i128,
    ) -> Result<(), Error> {
        stream.withdrawn = stream
            .withdrawn
            .checked_add(payout)
            .ok_or(Error::Overflow)?;

        // `Cancelled` is sticky: draining a cancelled stream to zero leaves it
        // visibly cancelled rather than relabelling it as a clean completion.
        if stream.withdrawn >= stream.deposited && stream.status != StreamStatus::Cancelled {
            stream.status = StreamStatus::Depleted;

            // A stream can be paused *after* maturity and then drained, and
            // depletion is terminal — `resume` would be rejected — so leaving
            // `paused_at` set would strand the stream in a state that says
            // "Depleted" and "frozen" at once, with nothing able to clear it.
            // Close the pause out here, exactly as `cancel` does. Accrual is
            // unaffected: reaching `withdrawn == deposited` means the stream
            // had already fully vested.
            if let Some(paused_at) = stream.paused_at {
                let now = env.ledger().timestamp();
                stream.paused_total = stream
                    .paused_total
                    .checked_add(now.saturating_sub(paused_at))
                    .ok_or(Error::Overflow)?;
                stream.paused_at = None;
            }
        }

        let token = stream.token.clone();
        let recipient = stream.recipient.clone();
        // Accompany the outbound transfer with its accounting: the pool's
        // expected balance drops by exactly what the recipient receives.
        storage::debit_pool(env, &token, payout)?;
        storage::save_stream(env, stream_id, stream);

        token_transfer(
            env,
            &token,
            &env.current_contract_address(),
            MuxedAddress::from(recipient),
            &payout,
        )?;

        events::withdrawn(env, stream_id, stream, payout);
        Ok(())
    }
}

#[cfg(test)]
#[path = "test/mod.rs"]
mod test;
