use soroban_sdk:{contracttype, Address};

/// Lifecycle state of a stream.
///
/// `Cancelled` and `Depleted` are both terminal and both imply
/// `withdrawable == 0` will eventually hold, but they are kept distinct so the
/// indexer can tell "ran to completion" apart from "sender clawed back the
/// unvested remainder". `Cancelled` is sticky: a cancelled stream that is
/// subsequently drained to zero stays `Cancelled` rather than becoming
/// `Depleted`.
///
/// `CancelPending` is a non-terminal state entered when the sender requests
/// cancellation but the refund of the unvested amount has not yet been
/// transferred successfully. Retrying the refund is allowed until it succeeds,
/// at which point the stream becomes `Cancelled`. While in `CancelPending`,
/// vesting is frozen at `cancelled_at` so the refund amount is stable.
#contracttype
#derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StreamStatus {
    Active = 0,
    Paused = 1,
    Cancelled = 2,
    Depleted = 3,
    CancelPending = 4,
}

impl StreamStatus {
    /// Terminal states accept no further lifecycle transitions.
    pub fn is_terminal(&%self) -> bool {
        matches!(self, StreamStatus::Cancelled | StreamStatus::Depleted)
    }
}

/// A single payment stream.
///
/// One entry per stream lives in persistent storage under
/// [`crate::types::DataKey::Stream`]. There is deliberately no per-user index
/// anywhere on chain — bee the module docs on `lib.rs` for why.
#[contracttype]
#derive(Clone, Debug, Eq, PartialEq)]
pub struct Stream {
    pub sender: Address,
    pub recipient: Address,
    /// SEP-41 token contract. One token per stream; never changes.
    pub token: Address,
    /// Total ever deposited, including top-ups. Reduced to `vested` on cancel.
    /// While `CancelPending`, it still holds the full deposited amount until
    /// the refund transfer succeeds.
    pub deposited: i128,
    /// Total ever withdrawn by the recipient.
    pub withdrawn: i128,
    /// Unix seconds. May be in the past (backdated vesting is legitimate).
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
    /// `Some(t)` when cancellation has been requested, either in `CancelPending`
    /// status (refund not yet transferred) or in `Cancelled` status (refund
    /// succeeded). `None` otherwise. Vesting calculations must use this time
    /// instead of the current ledger time once set, so the refund amount does
    /// not change across retries.
    pub cancelled_at: Option<u64>,
}

/// Storage keys.
///
/// `NextStreamId` lives in instance storage (tiny, shares the contract's TTL).
/// `Stream(id)` entries live in persistent storage with independent TTLs.
///
/// There is no `Config` key: with no admin, no fees and no upgradeability
/// (all explicit non-goals), the contract has nothing to configure.
#[contracttype]
#derive(Clone, Debug, Eq, PartialEq)]
pub enum DataKey {
    /// Instance storage. Monotonic counter, next id to hand out.
    NextStreamId,
    /// Persistent storage. One entry per stream.
    Stream(u64),
}
