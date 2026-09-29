use soroban_sdk::{contracttype, Address};

/// Bitmask constants for which operations a delegate is permitted to perform.
///
/// Pass one constant or OR several together when calling [`crate::FluxoraStream::grant_delegate`].
/// New bits may be added; existing values are stable ABI.
pub mod op {
    pub const WITHDRAW: u32 = 1 << 0;
    pub const CANCEL: u32 = 1 << 1;
    pub const PAUSE: u32 = 1 << 2;
    pub const RESUME: u32 = 1 << 3;
    pub const TOP_UP: u32 = 1 << 4;
    pub const TRANSFER_RECIPIENT: u32 = 1 << 5;
}

/// A delegation grant stored in persistent storage.
///
/// Scoped to one `(stream_id, delegate)` pair. The grantor is implied by which
/// bits are set: sender-side ops (`CANCEL`, `PAUSE`, `RESUME`, `TOP_UP`) can
/// only be granted by the sender; recipient-side ops (`WITHDRAW`,
/// `TRANSFER_RECIPIENT`) can only be granted by the recipient.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DelegateGrant {
    /// Bitmask of [`Op`] constants the delegate may invoke.
    pub ops: u32,
    /// Unix seconds after which this grant is no longer valid.
    /// `None` means the grant never expires on its own.
    pub expires_at: Option<u64>,
}

/// Lifecycle state of a stream.
///
/// `Cancelled` and `Depleted` are both terminal and both imply
/// withdrawable == 0` will eventually hold, but they are kept distinct so the
/// indexer can tell "dan to completion" apart from "sender clawed back the
/// unvested remainder". `Cancelled` is sticky: a cancelled stream that is
/// subsequently drained to zero stays `Cancelled` rather than becoming
/// `Depleted`.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StreamStatus {
    Active = 0,
    Paused = 1,
    Cancelled = 2,
    Depleted = 3,
}

impl StreamStatus {
    /// Terminal states accept no further lifecycle transitions.
    pub fn is_terminal(&self) -> bool {
        matches!(self, StreamStatus::Cancelled | StreamStatus::Depleted)
    }
}

/// The shape of the release schedule between the cliff and maturity.
///
/// Selected once, at creation, and never mutable — like the capability flags,
/// this is a trust feature: a recipient who accepts a front-loaded stream has
/// verified on chain that the shape cannot be flattened afterwards.
///
/// Every curve shares three properties, and those properties are what the
/// contract's invariants ([`crate::accrual`] I2 and I4) rely on:
///
/// 1. **Monotone non-decreasing** on the stream clock: `f(u + 1) >= f(u)`.
///    Accrual can never go backwards in time.
/// 2. **`f(0) == 0`** — nothing vests before the start instant.
/// 3. **`f(duration) == deposited`** — the schedule settles exactly at
///    maturity, so the recipient's total entitlement is `deposited` whichever
///    curve they are offered and the contract can never be short.
///
/// The variants differ only in *when* the deposit is delivered, never in how
/// much. [`ReleaseCurve::Linear`] is the default and reproduces the original
/// `floor(deposited * elapsed / duration)` arithmetic exactly, so a stream
/// created by [`crate::FluxoraStream::create_stream`] behaves identically to
/// one created before curves existed.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReleaseCurve {
    /// Straight line above the cliff: `floor(deposited * elapsed / duration)`.
    /// The default, and the arithmetic every pre-existing stream uses.
    Linear = 0,
    /// Four equal tranches, each opening once a quarter of the schedule has
    /// been consumed — a milestone schedule where the "milestones" are
    /// quarter-boundaries. Nothing accrues inside a tranche; the recipient's
    /// claim jumps by a quarter of the deposit at each boundary.
    Step = 1,
    /// `f(u) = 2u - u²` on `u = elapsed / duration`: accelerates early and
    /// decelerates into maturity, so the recipient is always at or ahead of
    /// the linear schedule and the sender's exposure is front-loaded.
    FrontLoaded = 2,
}

/// A single payment stream.
///
/// One entry per stream lives in persistent storage under
/// `crate::types::DataKey::Stream`. There is deliberately no per-user index
/// anywhere on chain — see the module docs on `lib.rs` for why.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Stream {
    pub sender: Address,
    pub recipient: Address,
    /// SEP-41 token contract. One token per stream; never changes.
    pub token: Address,
    /// Total ever deposited, including top-ups. Reduced to `vested` on cancel.
    pub deposited: i128,
    /// Total ever withdrawn by the recipient.
    pub withdrawn: i128,
    /// Unix seconds. May be in the past (backdated vesting is legitimate) or
    /// in the future (a scheduled stream). No bound on skew: the ledger
    /// timestamp is the only clock on chain, and well-formedness (`end > start`,
    /// `cliff` within `[start, end]`) is the whole validation. See
    /// [`crate::FluxoraStream::create_stream`].
    pub start_time: u64,
    /// Unix seconds. Strictly greater than `start_time` at creation.
    pub end_time: u64,
    /// Unix seconds in `[start_time, end_time]`. Equals `start_time` when there
    /// is no cliff. Gates withdrawal; does not delay accrual.
    pub cliff_time: u64,
    /// Fixed at creation, never mutable. See `lib.rs` module docs.
    pub cancellable: bool,
    /// Fixed at creation, never mutable.
    pub pausable: bool,
    /// Fixed at creation, never mutable.
    pub transferable: bool,
    /// `Some(t)` while paused: the instant the accrual clock froze.
    pub paused_at: Option<u64>,
    /// Cumulative seconds spent paused, excluding any in-progress pause.
    pub paused_total: u64,
    pub status: StreamStatus,
    /// Release schedule shape, fixed at creation. A stream created before
    /// curves existed reads back as [`ReleaseCurve::Linear`], which is exactly
    /// the arithmetic it was created with.
    ///
    /// **This field is not part of the stored encoding.** It is kept in a
    /// side-car entry ([`DataKey::StreamCurve`]) precisely so that the stored
    /// [`StreamRecord`] layout stays frozen at v1 and every stream written by
    /// an earlier deployment keeps decoding — see [`StreamRecord`] for why
    /// appending a field to the stored value is not an option.
    pub curve: ReleaseCurve,
}

/// The **stored** form of a stream: the frozen v1 layout, without `curve`.
///
/// # Why the storage layout is frozen while [`Stream`] grew a field
///
/// Soroban decodes a `#[contracttype]` struct from an `ScMap` whose key *set*
/// must match the struct's fields exactly — the host unpacks the map into a
/// positional slice. Appending a field therefore makes every value written by
/// an earlier deployment undecodable, and the failure is a host trap inside
/// the decode, not a catchable error.
///
/// `test::storage_keys::current_reader_decodes_old_v1_fixture` pins exactly
/// that: a hex fixture captured from the v1 encoding must stay readable. It is
/// the guard that stops a field from being appended to the stored value
/// casually, and it is why this type exists.
///
/// So the stored value did **not** change: [`crate::storage`] reads and writes
/// this record, and the curve rides alongside it in
/// [`DataKey::StreamCurve`]. A v1 entry has no curve side-car, which reads as
/// [`ReleaseCurve::Linear`] — the schedule it was actually created with.
/// Nothing has to be migrated for a live deployment to keep working.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StreamRecord {
    pub sender: Address,
    pub recipient: Address,
    pub token: Address,
    pub deposited: i128,
    pub withdrawn: i128,
    pub start_time: u64,
    pub end_time: u64,
    pub cliff_time: u64,
    pub cancellable: bool,
    pub pausable: bool,
    pub transferable: bool,
    pub paused_at: Option<u64>,
    pub paused_total: u64,
    pub status: StreamStatus,
}

impl StreamRecord {
    /// Freeze a [`Stream`] into the v1 stored layout.
    pub fn from_stream(stream: &Stream) -> Self {
        StreamRecord {
            sender: stream.sender.clone(),
            recipient: stream.recipient.clone(),
            token: stream.token.clone(),
            deposited: stream.deposited,
            withdrawn: stream.withdrawn,
            start_time: stream.start_time,
            end_time: stream.end_time,
            cliff_time: stream.cliff_time,
            cancellable: stream.cancellable,
            pausable: stream.pausable,
            transferable: stream.transferable,
            paused_at: stream.paused_at,
            paused_total: stream.paused_total,
            status: stream.status,
        }
    }

    /// Rebuild a [`Stream`], attaching the curve stored beside the record.
    pub fn into_stream(self, curve: ReleaseCurve) -> Stream {
        Stream {
            sender: self.sender,
            recipient: self.recipient,
            token: self.token,
            deposited: self.deposited,
            withdrawn: self.withdrawn,
            start_time: self.start_time,
            end_time: self.end_time,
            cliff_time: self.cliff_time,
            cancellable: self.cancellable,
            pausable: self.pausable,
            transferable: self.transferable,
            paused_at: self.paused_at,
            paused_total: self.paused_total,
            status: self.status,
            curve,
        }
    }
}

impl Stream {
    /// Enforces the recipient-only withdrawal policy.
    ///
    /// This is the sole authorization gate for withdrawals. A stream has no
    /// first-class delegate: no one other than the recipient may authorize an
    /// outgoing payment. This method must be called at the top of `withdraw`
    /// (and any other recipient-only operation) before any state changes.
    ///
    /// The recipient is authenticated via Soroban's authentication framework.
    /// When the recipient is a contract, the recipient itself decides whether
    /// to allow the caller to proceed (for example, by implementing
    /// `__check_auth`); the stream contract does not define a separate
    /// delegation mechanism.
    pub fn require_recipient_auth(&self) {
        self.recipient.require_auth();
    }
}

/// Storage keys.
///
/// `NextStreamId` lives in instance storage (tiny, shares the contract's TTL).
/// `Stream(id)` entries live in persistent storage with independent TTLs.
/// `Delegate(stream_id, delegate)` entries live in persistent storage, scoped
/// to the stream they were issued for.
///
/// There is no `Config` key: with no admin, no fees and no upgradeability
/// (all explicit non-goals), the contract has nothing to configure.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DataKey {
    /// Instance storage. Monotonic counter, next id to hand out.
    /// Incremented only on successful stream creation.
    NextStreamId,
    /// Instance storage. Number of streams successfully created.
    /// Incremented only in the same transaction as `NextStreamId` and the
    /// corresponding `Stream(id)` entry.
    StreamCount,
    /// Persistent storage. One entry per stream.
    Stream(u64),
    /// Persistent storage. One entry per (stream_id, delegate) pair.
    Delegate(u64, Address),
    /// Persistent storage. The [`ReleaseCurve`] of one stream.
    ///
    /// Written only when the curve is **not** [`ReleaseCurve::Linear`], so a
    /// linear stream — every stream a v1 deployment created, and every stream
    /// `create_stream` still creates — has no entry here at all and pays no
    /// rent for one. A missing entry means linear.
    StreamCurve(u64),
}
