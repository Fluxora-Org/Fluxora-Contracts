# Changelog

All notable changes to Fluxora Contracts will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added
- (leave empty — to be filled as new work is merged)

### Changed
- (leave empty)

### Fixed
- (leave empty)

---

## [0.1.0] - 2026-03-28

### Added

#### Functions
- `init(token: Address, admin: Address) -> Result<(), ContractError>` (admin only)
  Initialize the contract with token address and admin. Must be called exactly once before any other operations.

- `create_stream(sender: Address, recipient: Address, deposit_amount: i128, rate_per_second: i128, start_time: u64, cliff_time: u64, end_time: u64) -> Result<u64, ContractError>`
  Create a new payment stream and transfer deposit from sender. Returns unique stream ID.

- `create_streams(sender: Address, streams: Vec<CreateStreamParams>) -> Result<Vec<u64>, ContractError>`
  Batch-create multiple streams in one atomic transaction. Returns vector of stream IDs.

- `pause_stream(stream_id: u64) -> Result<(), ContractError>` (sender only)
  Halt withdrawals for a stream. Accrual continues based on time.

- `resume_stream(stream_id: u64) -> Result<(), ContractError>` (sender only)
  Re-enable withdrawals for a paused stream.

- `cancel_stream(stream_id: u64) -> Result<(), ContractError>` (sender only)
  Terminate a stream and refund unstreamed tokens to sender. Recipient can still withdraw accrued amount.

- `withdraw(stream_id: u64) -> Result<i128, ContractError>` (recipient only)
  Withdraw accrued tokens to the recipient's address. Returns amount withdrawn.

- `withdraw_to(stream_id: u64, destination: Address) -> Result<i128, ContractError>` (recipient only)
  Withdraw accrued tokens to a specified destination address. Returns amount withdrawn.

- `batch_withdraw(recipient: Address, stream_ids: Vec<u64>) -> Result<Vec<BatchWithdrawResult>, ContractError>` (recipient only)
  Withdraw from multiple streams in one transaction. Returns vector of withdrawal results.

- `top_up_stream(stream_id: u64, funder: Address, added_amount: i128) -> Result<(), ContractError>`
  Add tokens to an existing stream's deposit. Any funder can add tokens.

- `update_rate_per_second(stream_id: u64, new_rate_per_second: i128) -> Result<(), ContractError>` (sender only)
  Change the streaming rate for an active stream.

- `shorten_stream_end_time(stream_id: u64, new_end_time: u64) -> Result<(), ContractError>` (sender only)
  Move stream end time earlier and refund excess deposit to sender.

- `extend_stream_end_time(stream_id: u64, new_end_time: u64) -> Result<(), ContractError>` (sender only)
  Move stream end time later to extend streaming duration.

- `close_completed_stream(stream_id: u64) -> Result<(), ContractError>`
  Remove a completed stream from storage. Anyone can call for cleanup.

- `cancel_stream_as_admin(stream_id: u64) -> Result<(), ContractError>` (admin only)
  Administrative override to cancel any stream, bypassing sender authorization.

- `pause_stream_as_admin(stream_id: u64) -> Result<(), ContractError>` (admin only)
  Administrative override to pause any stream, bypassing sender authorization.

- `resume_stream_as_admin(stream_id: u64) -> Result<(), ContractError>` (admin only)
  Administrative override to resume any paused stream, bypassing sender authorization.

- `set_admin(new_admin: Address) -> Result<(), ContractError>` (admin only)
  Rotate the contract admin address to a new address.

- `set_global_emergency_paused(paused: bool) -> Result<(), ContractError>` (admin only)
  Set or clear the global emergency pause flag. When true, blocks most user operations.

- `global_resume() -> Result<(), ContractError>` (admin only)
  Clear the global emergency pause flag and restore normal operations.

#### View Functions
- `calculate_accrued(stream_id: u64) -> Result<i128, ContractError>` (read-only)
  Calculate the total accrued amount at current ledger time.

- `get_withdrawable(stream_id: u64) -> Result<i128, ContractError>` (read-only)
  Get the withdrawable amount at current ledger time.

- `get_claimable_at(stream_id: u64, timestamp: u64) -> Result<i128, ContractError>` (read-only)
  Simulate claimable amount at an arbitrary timestamp.

- `get_config() -> Result<Config, ContractError>` (read-only)
  Get the contract configuration (token address and admin address).

- `get_global_emergency_paused() -> bool` (read-only)
  Check if the contract is in global emergency pause state.

- `get_stream_state(stream_id: u64) -> Result<Stream, ContractError>` (read-only)
  Get the complete stream data structure.

- `get_stream_count() -> u64` (read-only)
  Get the total number of streams created.

- `get_recipient_streams(recipient: Address) -> Vec<u64>` (read-only)
  Get all stream IDs for a recipient (sorted by stream_id).

- `get_recipient_stream_count(recipient: Address) -> u64` (read-only)
  Get the count of streams for a recipient.

- `version() -> u32` (read-only)
  Get the compile-time contract version number.

#### Events
- `StreamCreated { stream_id: u64, sender: Address, recipient: Address, deposit_amount: i128, rate_per_second: i128, start_time: u64, cliff_time: u64, end_time: u64 }`
  Emitted when a new stream is created.

- `Withdrawal { stream_id: u64, recipient: Address, amount: i128 }`
  Emitted when a recipient withdraws tokens.

- `WithdrawalTo { stream_id: u64, recipient: Address, destination: Address, amount: i128 }`
  Emitted when a recipient withdraws to a specified destination.

- `StreamEvent::Paused(stream_id: u64)`
  Emitted when a stream is paused.

- `StreamEvent::Resumed(stream_id: u64)`
  Emitted when a paused stream is resumed.

- `StreamEvent::StreamCancelled(stream_id: u64)`
  Emitted when a stream is cancelled.

- `StreamEvent::StreamCompleted(stream_id: u64)`
  Emitted when a stream reaches completion status.

- `StreamEvent::StreamClosed(stream_id: u64)`
  Emitted when a completed stream is closed and removed from storage.

- `RateUpdated { stream_id: u64, old_rate_per_second: i128, new_rate_per_second: i128, effective_time: u64 }`
  Emitted when a stream's rate is updated.

- `StreamEndShortened { stream_id: u64, old_end_time: u64, new_end_time: u64, refund_amount: i128 }`
  Emitted when a stream's end time is shortened.

- `StreamEndExtended { stream_id: u64, old_end_time: u64, new_end_time: u64 }`
  Emitted when a stream's end time is extended.

- `StreamToppedUp { stream_id: u64, added_amount: i128, new_total: i128, new_end_time: u64 }`
  Emitted when additional tokens are added to a stream.

- `GlobalEmergencyPauseChanged { paused: bool }`
  Emitted when the global emergency pause flag is toggled.

- `ContractPauseChanged { paused: bool }`
  Emitted when the contract creation pause flag is toggled.

- `GlobalResumed { resumed_at: u64 }`
  Emitted when the contract is resumed from global emergency pause.

#### Errors
- `ContractError::StreamNotFound` — Thrown when trying to access a stream that doesn't exist.

- `ContractError::InvalidState` — Thrown when operation is not valid for current stream state.

- `ContractError::InvalidParams` — Thrown when provided parameters are invalid (negative amounts, invalid time ranges, etc.).

- `ContractError::ContractPaused` — Thrown when global emergency pause is active and operation is blocked.

- `ContractError::StartTimeInPast` — Thrown when stream start_time is before current ledger timestamp.

- `ContractError::ArithmeticOverflow` — Thrown when arithmetic operations exceed safe limits.

- `ContractError::Unauthorized` — Thrown when caller lacks required authorization.

- `ContractError::AlreadyInitialised` — Thrown when trying to initialize an already initialized contract.

- `ContractError::InsufficientBalance` — Thrown when token balance or allowance is insufficient.

- `ContractError::InsufficientDeposit` — Thrown when deposit amount doesn't cover total streamable amount.

- `ContractError::StreamAlreadyPaused` — Thrown when trying to pause an already paused stream.

- `ContractError::StreamNotPaused` — Thrown when trying to resume a stream that isn't paused.

- `ContractError::StreamTerminalState` — Thrown when trying to modify a stream in terminal state (Completed or Cancelled).

#### Types
- `Config { token: Address, admin: Address }`
  Global contract configuration set during initialization.

- `StreamStatus { Active = 0, Paused = 1, Completed = 2, Cancelled = 3 }`
  Enumeration of possible stream states.

- `Stream { stream_id: u64, sender: Address, recipient: Address, deposit_amount: i128, rate_per_second: i128, start_time: u64, cliff_time: u64, end_time: u64, withdrawn_amount: i128, status: StreamStatus, cancelled_at: Option<u64> }`
  Complete stream data structure stored in persistent storage.

- `CreateStreamParams { recipient: Address, deposit_amount: i128, rate_per_second: i128, start_time: u64, cliff_time: u64, end_time: u64 }`
  Parameters for creating new streams, used in batch operations.

- `BatchWithdrawResult { stream_id: u64, amount: i128 }`
  Per-stream result for batch withdrawal operations.

### Changed
- (nothing for initial release)

### Fixed
- (nothing for initial release)

### Security
- All state-mutating functions require appropriate authorization via require_auth()
- Admin functions are restricted to contract admin address set during init
- Sender functions require authorization from stream sender
- Recipient functions require authorization from stream recipient
- CEI (Checks-Effects-Interactions) pattern used to reduce reentrancy risk
- Global emergency pause allows admin to block most user operations in emergencies
- Token transfers use centralized pull_token/push_token helpers for security review

---

[Unreleased]: https://github.com/Fluxora-Org/Fluxora-Contracts/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/Fluxora-Org/Fluxora-Contracts/releases/tag/v0.1.0
