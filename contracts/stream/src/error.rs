use soroban_sdk::contracterror;

/// Every failure mode in Fluxora is a typed error. Nothing panics on a numeric
/// edge case: all arithmetic is checked and maps to [`Error::Overflow`].
///
/// Discriminants are part of the public ABI. Never renumber an existing
/// variant; only append.
///
/// ## Creation atomicity
/// Stream creation is transactional: `next_stream_id` and `stream_count` are
/// only mutated after all validation and the token transfer succeed. If any
/// phase fails, no ID is consumed and no count is incremented; stream IDs are
/// therefore contiguous with no gaps.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    // --- Lookup ---
    /// No stream exists with the given id.
    StreamNotFound = 1,

    // --- Creation validation ---
    /// `end_time <= start_time`. A zero or negative duration would divide by zero.
    InvalidTimeRange = 2,
    /// `cliff_time` is outside [start_time, end_time].
    InvalidCliff = 3,
    /// Deposit is zero or negative.
    InvalidDeposit = 4,
    /// `deposited < duration`, so the per-second rate truncates to zero and the
    /// recipient would accrue nothing. See `MIN_RATE_STROOPS_PER_SECOND`.
    DepositRateTooLow = 5,
    /// Sender and recipient are the same address.
    SelfStream = 6,

    // --- Authorization / capability ---
    /// Caller is not the party allowed to perform this action.
    Unauthorized = 7,
    /// `cancel` called on a stream created with `cancellable == false`.
    NotCancellable = 8,
    /// `pause` called on a stream created with `pausable == false`.
    NotPausable = 9,
    /// `transfer_recipient` called on a stream created with `transferable == false`.
    NotTransferable = 10,

    // --- State machine ---
    /// Action requires an `Active` stream.
    ///
    /// Reserved in the frozen ABI; current entry points use the more specific
    /// [`Self::StreamNotPaused`] / [`Self::StreamAlreadyPaused`] /
    /// [`Self::StreamTerminated`] variants instead. Do not renumber.
    StreamNotActive = 11,
    /// `resume` called on a stream that is not `Paused`.
    StreamNotPaused = 12,
    /// `pause` called on a stream that is already `Paused`.
    StreamAlreadyPaused = 13,
    /// Action attempted on a `Cancelled` or `Depleted` stream.
    StreamTerminated = 14,
    /// `top_up` on a stream whose accrual clock has already reached `end_time`,
    /// Topping up a matured stream would make the new funds instantly
    /// withdrawable; create a new stream instead.
    StreamMatured = 15,

    // --- Withdrawal ---
    /// Requested amount exceeds the currently withdrawable balance.
    InsufficientWithdrawable = 16,
    /// Withdrawable balance is zero.
    NothingToWithdraw = 17,
    /// Explicit withdraw amount was zero or negative.
    InvalidAmount = 18,

    // --- Resource limits ---
    /// Batch size exceeds `MAX_BATCH_SIZE`. Chunk client-side.
    BatchTooLarge = 19,
    /// Batch contained no stream ids.
    EmptyBatch = 20,
    /// A Batch referenced the same stream id more than once.
    DuplicateStreamId = 21,

    // --- Arithmetic ---
    /// A Checked arithmetic operation overflowed or underflowed.
    Overflow = 22,
    /// `top_up` amount is smaller than one second of streaming at the current
    /// rate, so it cannot extend the duration at all and would instead vest
    /// retroactively. Top up by at least `deposited / duration`.
    TopUpTooSmall = 23,

    // --- Transfer ---
    /// `transfer_recipient` to the current recipient.
    RepeatedTransfer = 24,
}
