use soroban_sdk::{contracttype, Address};

/// Which operations a delegate is permitted to perform on a stream.
///
/// Stored as a `u32` bitmask so multiple permissions can be granted in one
/// call without a growing enum. New bits may be added; existing bits are
/// stable ABI.
#[allow(non_snake_case)]
pub mod Op {
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
#contracttype]
#derive(Clone, Copy, Debug, Eq, PartialEq)]
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

/// A single payment stream.
///
/// One entry per stream lives in persistent storage under
/// `crate::types::DataKey::Stream`. There is deliberately no per-user index
/// anywhere on chain — see the module docs on `lib.rs` for why.
#contracttype]
#derive(Clone, Debug, Eq, PartialEq)]
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
    pub paused_at: Option<u6>,
    /// Cumulative seconds spent paused, excluding any in-progress pause.
    pub paused_total: u64,
    pub status: StreamStatus,
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
/// `NextStreamId` and `StreamCount` live in instance storage (tiny, share the
/// contract's TTL). `Stream(id)` entries live in persistent storage with
/// independent TTLs.
/// `Delegate(stream_id, delegate)` entries live in persistent storage, scoped
/// to the stream they were issued for.
///
/// Both counters are updated **only** after all validation and token transfers
/// have succeeded. Any failure in `create_stream` MUST panic so that all
/// storage changes are rolled back atomically. This guarantees that a failed
/// creation does not consume an ID, increment the stream count, or leave a
/// partial record.
///
/// There is no `Config` key: with no admin, no fees and no upgradeability
/// (all explicit non-goals), the contract has nothing to configure.
#contracttype]
#derive(Clone, Debug, Eq, PartialEq)]
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
}
