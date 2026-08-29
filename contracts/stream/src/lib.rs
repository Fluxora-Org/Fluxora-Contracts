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
//! schedule field, so an indexer can answer "show me my streams" without the
//! contract paying rent to remember.
//!
//! ## Immutable guarantees
//!
//! `cancellable`, `pausable` and `transferable` are fixed at creation and can
//! never change afterwards. This is a trust feature: before accepting a stream a
//! recipient can verify that the sender cannot claw it back, freeze it, or
//! reassign it. A stream that could *become* cancellable later would be
//! worthless as a guarantee.
//!
//! For the same reason the contract has no admin key, no upgrade path, no fee
//! switch and no global pause. Immutability is what lets another protocol depend
//! on this one.

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

pub use accrual::{
    cliff_reached, duration, elapsed, liability, refundable, stream_time, vested, withdrawable,
};
pub use error::Error;
pub use storage::{MIN_STREAM_TTL_LEDGERS, SECONDS_PER_LEDGER, TTL_BUFFER_SECONDS};
pub use types::op;
pub use types::{DataKey, DelegateGrant, Stream, StreamStatus};

use soroban_sdk::{
    contract, contractimpl, token, Address, Env, InvokeError, MuxedAddress, TryFromVal, Vec,
};

/// Maximum number of streams one batch call may touch.
///
/// # Where this number comes from
///
/// Measured, not guessed. `test::resource_limits` reports the real cost of a
/// full batch against protocol 27's mainnet limits, and the constraint that
/// binds is not the one you would expect:
///
/// Measured evidence from the release resource suite (20-stream batch):
///
/// | limit | used by a 20-stream batch | ceiling |
/// |---|---|---|
/// | total footprint (entries) | 51 | 400 |
/// | write entries | 24 | 200 |
/// | instructions | ~5.8M | 400M |
/// | **contract event bytes** | **10,240** | **16,384** |
///
/// Entry counts would allow well over a hundred streams per call. The *event
/// budget* allows about 32, because each stream emits a `withdrawn` event plus
/// the token contract's own `transfer` event — roughly 512 bytes per stream
/// between them.
///
/// Sixteen is that measured ceiling with a 2x safety factor. The margin is not
/// decoration: the per-stream event cost depends on the *token's* event
/// payload, and a token heavier than the Stellar Asset Contract used in the
/// tests would inflate it. A cap that merely fits today would fail on somebody
/// else's token.
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
pub const ABI_VERSION: u32 = 1;

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
    /// # Schedule
    ///
    /// Tokens accrue linearly from `start_time` to `end_time`. `start_time` may
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
    /// `cliff_time` **gates** the payout, it does not delay accrual. Pass
    /// `cliff_time == start_time` for no cliff. At the cliff instant the
    /// recipient becomes entitled to everything accrued since `start_time`, not
    /// merely what accrues after the cliff. This is standard vesting semantics
    /// and it surprises people, so it is worth restating in any UI.
    ///
    /// # Errors
    ///
    /// * [`Error::SelfStream`] — sender and recipient are the same address.
    /// * [`Error::InvalidDeposit`] — deposit is not positive.
    /// * [`Error::InvalidTimeRange`] — `end_time <= start_time`.
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
    ) -> Result<u64, Error> {
        sender.require_auth();

        if sender == recipient {
            return Err(Error::SelfStream);
        }
        if deposit <= 0 {
            return Err(Error::InvalidDeposit);
        }
        if end_time <= start_time {
            return Err(Error::InvalidTimeRange);
        }
        if cliff_time < start_time || cliff_time > end_time {
            return Err(Error::InvalidCliff);
        }

        let total_duration = end_time - start_time;

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
            cancellable,
            pausable,
            transferable,
            paused_at: None,
            paused_total: 0,
            status: StreamStatus::Active,
        };

        // Pull the deposit before writing the stream entry. If the token
        // transfer fails (missing contract, authorization refused, insufficient
        // balance) we return a typed error and leave no phantom entry in
        // storage — the id counter has already advanced, so the id is
        // consumed, but no stream with that id is observable to any caller.
        //
        // The sender's auth on this invocation covers the nested token
        // transfer; no prior approval is needed.
        token_transfer(
            &env,
            &token,
            &sender,
            MuxedAddress::from(env.current_contract_address()),
            &deposit,
        )?;

        storage::save_stream(&env, stream_id, &stream);
        storage::extend_instance(&env);

        events::stream_created(&env, stream_id, &stream);
        Ok(stream_id)
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
    /// # Errors
    ///
    /// * [`Error::StreamMatured`] — the accrual clock has already reached
    ///   `end_time`. Extending a matured stream would make the new funds
    ///   instantly (or near-instantly) withdrawable, which is never what the
    ///   sender means. Create a new stream instead.
    /// * [`Error::StreamTerminated`] — stream is cancelled or depleted.
    pub fn top_up(env: Env, stream_id: u64, amount: i128) -> Result<(), Error> {
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

        // delta = floor(amount * duration / deposited), preserving the rate.
        // Floor, never ceiling — see the rounding note above.
        let scaled = amount
            .checked_mul(current_duration)
            .ok_or(Error::Overflow)?;
        let delta = scaled
            .checked_div(stream.deposited)
            .ok_or(Error::Overflow)?;
        if delta < 0 || delta > u64::MAX as i128 {
            return Err(Error::Overflow);
        }

        // A top-up too small to buy even one second cannot extend the schedule,
        // so the only way to absorb it would be to raise the rate — which
        // re-vests elapsed time retroactively, the exact thing this function
        // exists to avoid. Reject instead.
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

        // Re-establish the creation-time guards against the new figures.
        new_deposited
            .checked_mul(new_duration as i128)
            .ok_or(Error::Overflow)?;
        if new_deposited < new_duration as i128 {
            return Err(Error::DepositRateTooLow);
        }

        let token = stream.token.clone();
        let sender = stream.sender.clone();

        token_transfer(
            &env,
            &token,
            &sender,
            MuxedAddress::from(env.current_contract_address()),
            &amount,
        )?;

        stream.deposited = new_deposited;
        stream.end_time = new_end;
        storage::save_stream(&env, stream_id, &stream);

        events::topped_up(&env, stream_id, &stream, amount);
        Ok(())
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
    /// # Errors
    ///
    /// * [`Error::StreamNotFound`] — no stream with this id.
    /// * [`Error::NotCancellable`] — created with `cancellable == false`.
    /// * [`Error::StreamTerminated`] — already cancelled or depleted.
    pub fn cancel(env: Env, stream_id: u64) -> Result<(), Error> {
        let mut stream = storage::load_stream(&env, stream_id)?;
        stream.sender.require_auth();

        if !stream.cancellable {
            return Err(Error::NotCancellable);
        }
        if stream.status.is_terminal() {
            return Err(Error::StreamTerminated);
        }

        let now = env.ledger().timestamp();
        let vested_now = accrual::vested(&stream, now)?;
        let refund = accrual::refundable(&stream, now)?;

        // Issue #1584 — the accounting the `Cancelled` event publishes.
        //
        // `vested_now` is the *total* vested at this instant, cumulative and
        // inclusive of `withdrawn`; `refund` is the unvested remainder handed
        // back to the sender. Conservation (invariant I4) says the two must
        // partition the pre-cancel deposit exactly, with nothing created or
        // destroyed in between. Checked here rather than trusted, because this
        // is the identity every downstream ledger reconciles against.
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
        let settle_at = accrual::stream_time(&stream, now).max(stream.start_time);

        stream.deposited = vested_now;
        stream.end_time = settle_at;
        stream.paused_at = None;
        stream.status = StreamStatus::Cancelled;

        let token = stream.token.clone();
        let sender = stream.sender.clone();
        storage::save_stream(&env, stream_id, &stream);

        if refund > 0 {
            token_transfer(
                &env,
                &token,
                &env.current_contract_address(),
                MuxedAddress::from(sender),
                &refund,
            )?;
        }

        // Issue #1584: the event's `vested` is read off the settled stream by
        // the helper (`stream.deposited`, set above), so it cannot disagree with
        // storage. Asserted here so the intent survives a future edit.
        debug_assert_eq!(stream.deposited, vested_now);
        events::cancelled(&env, stream_id, &stream, refund);
        Ok(())
    }

    /// Pause accrual. Only the sender, and only if `pausable`.
    ///
    /// Pausing freezes the stream's clock and pushes the effective end date
    /// forward by the paused duration. Total value delivered stays constant; the
    /// schedule simply stretches. The recipient can still withdraw what they
    /// already earned.
    pub fn pause(env: Env, stream_id: u64) -> Result<(), Error> {
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
        stream.paused_at = Some(now);
        stream.status = StreamStatus::Paused;
        storage::save_stream(&env, stream_id, &stream);

        events::paused(&env, stream_id, &stream, now);
        Ok(())
    }

    /// Resume a paused stream, absorbing the paused interval into
    /// `paused_total` so the clock picks up exactly where it stopped.
    pub fn resume(env: Env, stream_id: u64) -> Result<(), Error> {
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

    /// Reassign a stream's entire remaining claim to a new recipient.
    /// Recipient auth.
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
    pub fn transfer_recipient(
        env: Env,
        stream_id: u64,
        new_recipient: Address,
    ) -> Result<(), Error> {
        let mut stream = storage::load_stream(&env, stream_id)?;
        stream.recipient.require_auth();

        if !stream.transferable {
            return Err(Error::NotTransferable);
        }
        if stream.status == StreamStatus::Depleted {
            return Err(Error::StreamTerminated);
        }
        if new_recipient == stream.sender {
            return Err(Error::SelfStream);
        }

        let old_recipient = stream.recipient.clone();
        if old_recipient == new_recipient {
            return Ok(());
        }

        stream.recipient = new_recipient.clone();
        storage::save_stream(&env, stream_id, &stream);

        events::recipient_transferred(&env, stream_id, &old_recipient, &new_recipient);
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
    /// Takes effect immediately — the delegate's next call will be rejected.
    /// Funds the delegate has already moved (e.g. via a prior `withdraw`) are
    /// unaffected: revocation only stops future invocations.
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
        Ok(payout)
    }

    /// Cancel as a delegate. Requires [`op::CANCEL`] grant.
    pub fn delegate_cancel(env: Env, stream_id: u64, delegate: Address) -> Result<(), Error> {
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
            token_transfer(
                &env,
                &token,
                &env.current_contract_address(),
                MuxedAddress::from(sender),
                &refund,
            )?;
        }

        events::cancelled(&env, stream_id, &stream, refund, vested_now);
        Ok(())
    }

    /// Pause as a delegate. Requires [`op::PAUSE`] grant.
    pub fn delegate_pause(env: Env, stream_id: u64, delegate: Address) -> Result<(), Error> {
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
        Self::check_delegate(&env, stream_id, &delegate, op::TOP_UP)?;
        let mut stream = storage::load_stream(&env, stream_id)?;
        // A delegate may request a top-up, but only the sender may authorize
        // spending the sender's tokens.
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

        new_deposited
            .checked_mul(new_duration as i128)
            .ok_or(Error::Overflow)?;
        if new_deposited < new_duration as i128 {
            return Err(Error::DepositRateTooLow);
        }

        let token = stream.token.clone();
        stream.deposited = new_deposited;
        stream.end_time = new_end;
        storage::save_stream(&env, stream_id, &stream);

        // Tokens come from the sender — require their auth even though a
        // delegate triggered this call.
        sender.require_auth();
        token_transfer(
            &env,
            &token,
            &delegate,
            MuxedAddress::from(env.current_contract_address()),
            &amount,
        )?;

        events::topped_up(&env, stream_id, &stream, amount);
        Ok(())
    }

    /// Transfer recipient as a delegate. Requires [`op::TRANSFER_RECIPIENT`] grant.
    pub fn delegate_transfer_recipient(
        env: Env,
        stream_id: u64,
        delegate: Address,
        new_recipient: Address,
    ) -> Result<(), Error> {
        Self::check_delegate(&env, stream_id, &delegate, op::TRANSFER_RECIPIENT)?;
        let mut stream = storage::load_stream(&env, stream_id)?;

        if !stream.transferable {
            return Err(Error::NotTransferable);
        }
        if stream.status == StreamStatus::Depleted {
            return Err(Error::StreamTerminated);
        }
        if new_recipient == stream.sender {
            return Err(Error::SelfStream);
        }

        let old_recipient = stream.recipient.clone();
        if old_recipient == new_recipient {
            return Ok(());
        }

        stream.recipient = new_recipient.clone();
        storage::save_stream(&env, stream_id, &stream);

        events::recipient_transferred(&env, stream_id, &old_recipient, &new_recipient);
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
    pub fn extend_stream_ttl(env: Env, stream_id: u64) -> Result<u32, Error> {
        // Authorization: permissionless by design — see doc comment. Any caller
        // may pay rent for any stream. The caller has no address parameter and
        // no `require_auth` is invoked.
        let stream = storage::peek_stream(&env, stream_id)?;
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
    pub fn batch_extend_ttl(env: Env, stream_ids: Vec<u64>) -> Result<u32, Error> {
        // Authorization: permissionless — same policy as `extend_stream_ttl`.
        let stream_ids = Self::validate_batch_ids(&env, &stream_ids)?;
        Self::reject_duplicate_ids(&stream_ids)?;

        let mut extended = 0u32;
        for stream_id in stream_ids.iter() {
            if let Ok(stream) = storage::peek_stream(&env, stream_id) {
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
    // Internal
    // ---------------------------------------------------------------------

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
