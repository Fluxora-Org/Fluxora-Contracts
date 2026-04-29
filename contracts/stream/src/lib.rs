#![no_std]
#![allow(clippy::too_many_arguments)]

mod accrual;
#[cfg(test)]
mod checksum;

use soroban_sdk::{
    contract, contractimpl, contracttype, symbol_short, token, xdr::ToXdr, Address, Bytes,
    BytesN, Env,
};

// ---------------------------------------------------------------------------
// TTL constants
// ---------------------------------------------------------------------------

/// Minimum remaining TTL (in ledgers) before we bump.  ~1 day at 5 s/ledger.
const INSTANCE_LIFETIME_THRESHOLD: u32 = 17_280;
/// Extend to ~7 days of ledgers when bumping instance storage.
const INSTANCE_BUMP_AMOUNT: u32 = 120_960;
/// Minimum remaining TTL for persistent (stream) entries.
const PERSISTENT_LIFETIME_THRESHOLD: u32 = 17_280;
/// Extend persistent entries to ~7 days of ledgers.
const PERSISTENT_BUMP_AMOUNT: u32 = 120_960;

// ---------------------------------------------------------------------------
// Pagination limits (DoS prevention)
// ---------------------------------------------------------------------------

/// Maximum page size for paginated export views.
///
/// Prevents unbounded memory usage and gas exhaustion when exporting stream data.
/// All paginated entrypoints enforce this limit strictly.
pub const MAX_PAGE_SIZE: u64 = 100;

<<<<<<< HEAD
=======
/// Maximum memo payload size in bytes (stream metadata for indexers).
pub const MAX_MEMO_BYTES: usize = 64;

/// Maximum schedule templates a single owner may register (bounds `OwnerTemplateIds` growth).
pub const MAX_TEMPLATES_PER_OWNER: u32 = 64;
/// Global bound on stored schedule templates (DoS / storage bloat prevention).
pub const MAX_GLOBAL_TEMPLATES: u64 = 10_000;

>>>>>>> upstream/main
// Contract version
// ---------------------------------------------------------------------------

/// Compile-time contract version number.
///
/// This constant is embedded in the WASM binary at compile time and returned
/// by the permissionless `version()` entry-point. It is the single source of
/// truth that integrators, deployment scripts, and indexers use to detect
/// which protocol revision is running on-chain.
///
/// # Versioning policy
///
/// | Change type | Action required |
/// |-------------|-----------------|
/// | Breaking ABI change (renamed/removed entry-point, changed parameter order, changed error codes, changed event shape) | Increment `CONTRACT_VERSION` |
/// | New entry-point that is purely additive (old clients can ignore it) | Increment `CONTRACT_VERSION` (conservative; recommended) |
/// | Internal refactor with identical external behaviour | No increment required |
/// | Documentation-only change | No increment required |
///
/// ## What counts as breaking
/// - Removing or renaming a public function
/// - Changing the type or order of any function parameter
/// - Changing a `ContractError` discriminant value
/// - Changing the shape of an emitted event payload (`StreamCreated`, `Withdrawal`, etc.)
/// - Changing storage key layout in a way that makes existing persistent entries unreadable
///
/// ## What does NOT require an increment
/// - Adding a new public function (additive)
/// - Tightening validation (e.g. rejecting a previously-accepted edge case) — but document it
/// - Gas optimisations with identical observable behaviour
/// - Changing TTL bump constants
///
/// # Migration notes for operators
///
/// Soroban contracts are **not upgradeable in-place** by default. A new version means:
/// 1. Deploy a new contract instance (new `CONTRACT_ID`).
/// 2. Call `init` on the new instance with the same token and admin.
/// 3. Migrate active streams off-chain: cancel or let them complete on the old instance,
///    then recreate on the new instance if needed.
/// 4. Update all integrations (wallets, indexers, treasury tooling) to point at the new
///    `CONTRACT_ID` and verify `version()` returns the expected value before use.
/// 5. Announce the migration with sufficient lead time so recipients can withdraw
///    accrued funds from the old instance before it is abandoned.
///
/// There is no on-chain migration path between versions. All stream state is local to
/// the contract instance that created it.
///
/// # Residual risk
/// - If an operator forgets to increment this constant before deploying a breaking change,
///   integrators will not detect the incompatibility until a runtime failure occurs.
///   Code review and CI checks on this constant are the primary safeguard.
///
/// Bumped to 5: `withdraw_dust_threshold: i128` added to `Stream` struct and creation params
/// to reduce fee/event spam from tiny withdrawals.
pub const CONTRACT_VERSION: u32 = 6;

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

/// Global configuration for the Fluxora protocol.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Config {
    pub token: Address,
    pub admin: Address,
}

#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StreamStatus {
    Active = 0,
    Paused = 1,
    Completed = 2,
    Cancelled = 3,
}
#[soroban_sdk::contracterror]
#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum ContractError {
    StreamNotFound = 1,
    InvalidState = 2,
    InvalidParams = 3,
    /// Global emergency pause is active; stream creation is blocked.
    ContractPaused = 4,
    /// Start time is before the current ledger timestamp.
    StartTimeInPast = 5,
    /// Arithmetic overflow in stream calculations (e.g. deposit total).
    ArithmeticOverflow = 6,
    /// Caller is not authorized to perform this operation.
    Unauthorized = 7,
    /// Contract is already initialized.
    AlreadyInitialised = 8,
    /// Token balance or allowance is insufficient (emulated check if possible, otherwise caught by token client).
    InsufficientBalance = 9,
    /// Deposit amount does not cover the total streamable amount.
    InsufficientDeposit = 10,
    /// Stream is already in Paused state.
    StreamAlreadyPaused = 11,
    /// Stream is not in Paused state (e.g. trying to resume an Active stream).
    StreamNotPaused = 12,
    /// Stream is in a terminal state (Completed or Cancelled) and cannot be modified.
    StreamTerminalState = 13,
    /// Duplicate stream IDs were supplied to a batch operation.
    DuplicateStreamId = 14,
    /// No template exists for the given template id.
    TemplateNotFound = 15,
    /// Template registry limits exceeded (per-owner or global cap).
    TemplateLimitExceeded = 16,
    /// Caller is not the template owner for a protected template operation.
    TemplateUnauthorized = 17,
    /// The signature deadline has passed (current ledger time > deadline).
    SignatureDeadlineExpired = 18,
    /// The provided signature does not match the expected signer.
    InvalidSignature = 19,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StreamEvent {
    Paused(u64),
    Resumed(u64),
    StreamCancelled(u64),
    StreamCompleted(u64),
    StreamClosed(u64),
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct StreamCreated {
    pub stream_id: u64,
    pub sender: Address,
    pub recipient: Address,
    pub deposit_amount: i128,
    pub rate_per_second: i128,
    pub start_time: u64,
    pub cliff_time: u64,
    pub end_time: u64,
    /// Optional withdrawal threshold (raw units). Withdrawals below this
    /// amount are skipped unless they are the final drain or the stream is terminal.
    pub withdraw_dust_threshold: i128,
    /// Optional bounded memo for indexer correlation (e.g. payroll batch ID).
    /// `None` when no memo was supplied at creation time.
    pub memo: Option<soroban_sdk::Bytes>,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct Withdrawal {
    pub stream_id: u64,
    pub recipient: Address,
    pub amount: i128,
}

/// Emitted when a recipient withdraws to a specified destination via `withdraw_to`.
#[contracttype]
#[derive(Clone, Debug)]
pub struct WithdrawalTo {
    pub stream_id: u64,
    pub recipient: Address,
    pub destination: Address,
    pub amount: i128,
}

/// Emitted when a relayer executes a recipient-signed delegated withdrawal.
#[contracttype]
#[derive(Clone, Debug)]
pub struct DelegatedWithdrawal {
    pub stream_id: u64,
    pub recipient: Address,
    pub relayer: Address,
    pub destination: Address,
    pub amount: i128,
    pub nonce: u64,
}

/// Emitted when a recipient rotates their receiving address for a stream.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecipientUpdated {
    pub stream_id: u64,
    pub old_recipient: Address,
    pub new_recipient: Address,
}

/// Per-stream result for `batch_withdraw`.
#[contracttype]
#[derive(Clone, Debug)]
pub struct BatchWithdrawResult {
    pub stream_id: u64,
    pub amount: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WithdrawToParam {
    pub stream_id: u64,
    pub destination: Address,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct RateUpdated {
    pub stream_id: u64,
    pub old_rate_per_second: i128,
    pub new_rate_per_second: i128,
    /// Ledger timestamp when the rate update became effective.
    pub effective_time: u64,
}

/// Emitted when the sender safely decreases the streaming rate via `decrease_rate_per_second`.
///
/// The `checkpointed_amount` field records how many tokens were mathematically
/// accrued under the **old** rate at the moment of the rate change. The new rate
/// is applied only to the remaining stream duration from `effective_time` onward.
#[contracttype]
#[derive(Clone, Debug)]
pub struct RateDecreased {
    pub stream_id: u64,
    pub old_rate_per_second: i128,
    pub new_rate_per_second: i128,
    /// Ledger timestamp when the decrease became effective (== `checkpointed_at`).
    pub effective_time: u64,
    /// Accrued amount locked in at `effective_time` under the old rate.
    pub checkpointed_amount: i128,
    /// Tokens refunded to the sender: `old_deposit - new_max_payable`.
    pub refund_amount: i128,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct StreamEndShortened {
    /// Stream whose schedule was shortened.
    pub stream_id: u64,
    /// Previous `end_time` before this mutation.
    pub old_end_time: u64,
    /// New `end_time` after this mutation.
    pub new_end_time: u64,
    /// Tokens refunded to sender: `old_deposit_amount - new_deposit_amount`.
    pub refund_amount: i128,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct StreamEndExtended {
    pub stream_id: u64,
    pub old_end_time: u64,
    pub new_end_time: u64,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct StreamToppedUp {
    pub stream_id: u64,
    pub top_up_amount: i128,
    pub new_deposit_amount: i128,
    /// `end_time` after the top-up (unchanged by top-up itself; included so
    /// indexers can correlate with any subsequent `extend_stream_end_time` call).
    pub new_end_time: u64,
}

/// Emitted when the stream sender is rotated via `transfer_sender`.
///
/// The `old_sender` loses all sender-role privileges (pause, cancel, rate updates, etc.)
/// and the `new_sender` gains them immediately. Recipient entitlement is unchanged.
#[contracttype]
#[derive(Clone, Debug)]
pub struct SenderTransferred {
    pub stream_id: u64,
    pub old_sender: Address,
    pub new_sender: Address,
}

/// Emitted when the contract admin toggles the global emergency pause flag.
#[contracttype]
#[derive(Clone, Debug)]
pub struct GlobalEmergencyPauseChanged {
    pub paused: bool,
}

/// Emitted when the admin sweeps excess tokens from the contract.
#[contracttype]
#[derive(Clone, Debug)]
pub struct ExcessSwept {
    pub to: Address,
    pub amount: i128,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct GlobalResumed {
    pub resumed_at: u64,
}

/// Emitted when the contract admin toggles the creation-pause flag via `set_contract_paused`.
///
/// When `paused == true`, `create_stream` and `create_streams` revert with
/// `ContractError::ContractPaused`. All other operations are unaffected.
#[contracttype]
#[derive(Clone, Debug)]
pub struct ContractPauseChanged {
    pub paused: bool,
}

/// Emitted when the protocol is globally paused via `pause_protocol`.
#[contracttype]
#[derive(Clone, Debug)]
pub struct ProtocolPaused {
    pub reason: soroban_sdk::String,
    pub paused_at: u64,
}

/// Emitted when the protocol is globally resumed via `resume_protocol`.
#[contracttype]
#[derive(Clone, Debug)]
pub struct ProtocolResumed {
    pub resumed_at: u64,
}

/// Information about the current protocol pause state.
/// Returned by `get_pause_info()` query entrypoint.
#[contracttype]
#[derive(Clone, Debug)]
pub struct PauseInfo {
    pub is_paused: bool,
    pub reason: Option<soroban_sdk::String>,
    pub paused_at: Option<u64>,
    pub paused_by: Option<Address>,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Stream {
    pub stream_id: u64,
    pub sender: Address,
    pub recipient: Address,
    pub deposit_amount: i128,
    pub rate_per_second: i128,
    pub start_time: u64,
    pub cliff_time: u64,
    pub end_time: u64,
    pub withdrawn_amount: i128,
    pub status: StreamStatus,
    pub cancelled_at: Option<u64>,
    /// Total tokens mathematically accrued up to `checkpointed_at` under all
    /// previous rates. Updated by `decrease_rate_per_second` (and by
    /// `update_rate_per_second` for symmetry) so that the new rate applies only
    /// from `checkpointed_at` forward. Initialised to 0 at stream creation.
    pub checkpointed_amount: i128,
    /// Ledger timestamp of the last rate change (or `start_time` on creation).
    /// `calculate_accrued` uses this as the start of the current rate epoch.
    pub checkpointed_at: u64,
    /// Optional withdrawal threshold (raw units). Withdrawals below this
    /// amount are skipped unless they are the final drain or the stream is terminal.
    pub withdraw_dust_threshold: i128,
    /// Optional bounded memo for indexer correlation (e.g. payroll batch ID).
    /// Maximum length: `MAX_MEMO_BYTES` (64 bytes). `None` when not supplied.
    pub memo: Option<soroban_sdk::Bytes>,
}


/// Pagination result for recipient stream listing
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Page {
    /// Stream IDs for this page (sorted ascending)
    pub stream_ids: soroban_sdk::Vec<u64>,
    /// Next cursor for pagination (0 if no more pages)
    pub next_cursor: u64,
}

/// Reason code for pausing a stream.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PauseReason {
    /// Operational pause initiated by the stream sender.
    Operational = 0,
    /// Administrative pause initiated by the contract admin.
    Administrative = 1,
}

/// Emitted when a stream is paused.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StreamPaused {
    pub stream_id: u64,
    pub reason: PauseReason,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct CreateStreamParams {
    /// Address that will receive streamed tokens for this stream entry.
    pub recipient: Address,
    /// Total amount escrowed for this stream entry.
    pub deposit_amount: i128,
    /// Streaming speed in tokens per second for this stream entry.
    pub rate_per_second: i128,
    /// Ledger timestamp when accrual starts for this stream entry.
    pub start_time: u64,
    /// Ledger timestamp when withdrawals become enabled for this stream entry.
    pub cliff_time: u64,
    /// Ledger timestamp when accrual stops for this stream entry.
    pub end_time: u64,
    /// Optional withdrawal threshold (raw units) to reduce fee spam.
    pub withdraw_dust_threshold: Option<i128>,
    /// Optional bounded memo for indexer correlation (e.g. payroll batch ID).
    /// Maximum `MAX_MEMO_BYTES` (64) bytes. Pass `None` to omit.
    pub memo: Option<soroban_sdk::Bytes>,
}

/// Parameters for creating a payment stream with relative (offset-based) times.
///
/// Computes `start_time`, `cliff_time`, and `end_time` by adding offsets to the
/// current ledger timestamp (`env.ledger().timestamp()`). This eliminates off-chain
/// calculation errors that lead to `StartTimeInPast` failures.
///
/// # Time offsets
/// - `start_delay`: Seconds to add to current timestamp for stream start
/// - `cliff_delay`: Seconds to add to current timestamp for cliff time (must be >= start_delay)
/// - `duration`: Total duration of stream in seconds (end_time = start_time + duration)
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreateStreamRelativeParams {
    /// Address that will receive streamed tokens for this stream entry.
    pub recipient: Address,
    /// Total amount escrowed for this stream entry.
    pub deposit_amount: i128,
    /// Streaming speed in tokens per second for this stream entry.
    pub rate_per_second: i128,
    /// Delay (in seconds) before stream accrual starts, relative to current timestamp.
    pub start_delay: u64,
    /// Delay (in seconds) before withdrawals are allowed, relative to current timestamp.
    pub cliff_delay: u64,
    /// Total duration the stream runs (in seconds) from start_time to end_time.
    pub duration: u64,
    /// Optional withdrawal threshold (raw units) to reduce fee spam.
    pub withdraw_dust_threshold: Option<i128>,
    /// Optional bounded memo for indexer correlation (e.g. payroll batch ID).
    /// Maximum `MAX_MEMO_BYTES` (64) bytes. Pass `None` to omit.
    pub memo: Option<soroban_sdk::Bytes>,
}

/// Reusable relative schedule (offsets only). Amounts are supplied when creating a stream.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StreamScheduleTemplate {
    pub template_id: u64,
    pub owner: Address,
    pub start_delay: u64,
    pub cliff_delay: u64,
    pub duration: u64,
}

/// Namespace for all contract storage keys.
///
/// # Evolution policy
///
/// `DataKey` is a `#[contracttype]` enum. Soroban serialises enum variants by
/// their **discriminant index** (0-based, in declaration order). Changing the
/// order of existing variants, or inserting a new variant anywhere other than
/// the **end** of the enum, will silently shift all subsequent discriminants
/// and make every existing persistent storage entry unreadable.
///
/// Rules for contributors:
/// 1. **Never reorder** existing variants.
/// 2. **Never remove** a variant that has ever been written to a live network.
///    Mark it deprecated in a doc comment instead and stop writing to it.
/// 3. **Always append** new variants at the end of the enum.
/// 4. **Increment `CONTRACT_VERSION`** whenever a new variant is added or an
///    existing variant's associated type changes — both are breaking changes
///    for any off-chain tool that reads storage directly.
/// 5. Document the ledger at which each variant was first deployed so that
///    migration tooling can determine which entries exist on a given instance.
///
/// Current discriminant assignments (must never change) — see enum definition below for order.
#[contracttype]
pub enum DataKey {
    Config,                    // Instance storage for global settings (admin/token).
    NextStreamId,              // Instance storage for the auto-incrementing ID counter.
    Stream(u64),               // Persistent storage for individual stream data (O(1) lookup).
    RecipientStreams(Address), // Persistent storage for recipient stream index (sorted by stream_id).
    /// Global emergency pause flag (bool). This is a contract-wide circuit breaker.
    GlobalEmergencyPaused,
    /// Creation pause flag (bool). Appended to avoid shifting existing key discriminants.
    CreationPaused,
    /// Protocol pause reason (String). Human-readable reason for the pause.
    GlobalPauseReason,
    /// Protocol pause timestamp (u64). Ledger timestamp when pause was activated.
    GlobalPauseTimestamp,
    /// Protocol pause admin (Address). The admin address that activated the pause.
    GlobalPauseAdmin,
    /// Auto-claim destination per stream (Address). Set by recipient to redirect withdrawals.
    AutoClaimDestination(u64),
    /// Monotonic template id counter (`u64`, instance storage).
    NextTemplateId,
    /// Number of templates currently stored (`u64`, instance storage).
    ActiveTemplateCount,
    /// Registered relative schedule template (persistent).
    StreamTemplate(u64),
    /// Template ids owned by an address (persistent `Vec<u64>`; length capped).
    OwnerTemplateIds(Address),
    /// Sum of outstanding deposit liabilities (`i128`, instance storage).
    TotalLiabilities,
    /// Per-recipient nonce counter for delegated-withdraw replay protection.
    /// Appended last to preserve existing discriminant values.
    WithdrawNonce(Address),
}

// ---------------------------------------------------------------------------
// Storage helpers
// ---------------------------------------------------------------------------

/// Extend instance storage TTL so Config and NextStreamId do not expire.
/// Called on every entry-point that reads or writes instance storage.
fn bump_instance_ttl(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
}

fn get_config(env: &Env) -> Result<Config, ContractError> {
    bump_instance_ttl(env);
    env.storage()
        .instance()
        .get(&DataKey::Config)
        .ok_or(ContractError::InvalidState) // Not initialised
}

fn get_token(env: &Env) -> Result<Address, ContractError> {
    get_config(env).map(|c| c.token)
}

fn get_admin(env: &Env) -> Result<Address, ContractError> {
    get_config(env).map(|c| c.admin)
}

/// Returns whether the contract is in **global emergency pause** (default `false` if unset).
fn is_global_emergency_paused(env: &Env) -> bool {
    env.storage()
        .instance()
        .get(&DataKey::GlobalEmergencyPaused)
        .unwrap_or(false)
}

fn is_creation_paused(env: &Env) -> bool {
    env.storage()
        .instance()
        .get(&DataKey::CreationPaused)
        .unwrap_or(false)
}

/// Returns `Err(ContractError::ContractPaused)` when [`is_global_emergency_paused`] is true.
/// Admin/admin-override entrypoints must not call this so operators can still intervene.
fn require_not_globally_paused(env: &Env) -> Result<(), ContractError> {
    if is_global_emergency_paused(env) {
        return Err(ContractError::ContractPaused);
    }
    Ok(())
}

/// Blocks new stream creation when the emergency pause or creation-only pause is active.
fn require_not_creation_paused(env: &Env) -> Result<(), ContractError> {
    require_not_globally_paused(env)?;
    if is_creation_paused(env) {
        return Err(ContractError::ContractPaused);
    }
    Ok(())
}

/// Returns whether the protocol is globally paused (checks both GlobalEmergencyPaused and CreationPaused).
/// Default is false (not paused) if no pause keys are set.
fn is_protocol_paused(env: &Env) -> bool {
    is_global_emergency_paused(env) || is_creation_paused(env)
}

/// Get the stored pause reason, if any.
fn get_pause_reason(env: &Env) -> Option<soroban_sdk::String> {
    env.storage().instance().get(&DataKey::GlobalPauseReason)
}

/// Get the stored pause timestamp, if any.
fn get_pause_timestamp(env: &Env) -> Option<u64> {
    env.storage().instance().get(&DataKey::GlobalPauseTimestamp)
}

/// Get the stored pause admin address, if any.
fn get_pause_admin(env: &Env) -> Option<Address> {
    env.storage().instance().get(&DataKey::GlobalPauseAdmin)
}

fn read_stream_count(env: &Env) -> u64 {
    bump_instance_ttl(env);
    env.storage()
        .instance()
        .get(&DataKey::NextStreamId)
        .unwrap_or(0u64)
}

fn set_stream_count(env: &Env, count: u64) {
    env.storage().instance().set(&DataKey::NextStreamId, &count);
    bump_instance_ttl(env);
}

// ---------------------------------------------------------------------------
// Delegated-withdraw nonce helpers
// ---------------------------------------------------------------------------

/// Read the current nonce for a recipient (0 if never used).
fn read_withdraw_nonce(env: &Env, recipient: &Address) -> u64 {
    env.storage()
        .persistent()
        .get(&DataKey::WithdrawNonce(recipient.clone()))
        .unwrap_or(0u64)
}

/// Increment and persist the nonce for a recipient.
fn increment_withdraw_nonce(env: &Env, recipient: &Address) -> u64 {
    let next = read_withdraw_nonce(env, recipient) + 1;
    let key = DataKey::WithdrawNonce(recipient.clone());
    env.storage().persistent().set(&key, &next);
    env.storage().persistent().extend_ttl(
        &key,
        PERSISTENT_LIFETIME_THRESHOLD,
        PERSISTENT_BUMP_AMOUNT,
    );
    next
}

fn load_stream(env: &Env, stream_id: u64) -> Result<Stream, ContractError> {
    let key = DataKey::Stream(stream_id);
    let stream: Stream = env
        .storage()
        .persistent()
        .get(&key)
        .ok_or(ContractError::StreamNotFound)?;

    // Bump TTL on read so actively-queried streams don't expire
    env.storage().persistent().extend_ttl(
        &key,
        PERSISTENT_LIFETIME_THRESHOLD,
        PERSISTENT_BUMP_AMOUNT,
    );

    Ok(stream)
}

pub fn save_stream(env: &Env, stream: &Stream) {
    let key = DataKey::Stream(stream.stream_id);
    env.storage().persistent().set(&key, stream);
    env.storage().persistent().extend_ttl(
        &key,
        PERSISTENT_LIFETIME_THRESHOLD,
        PERSISTENT_BUMP_AMOUNT,
    );
}

fn is_terminal_state(env: &Env, stream: &Stream) -> bool {
    if stream.status == StreamStatus::Completed || stream.status == StreamStatus::Cancelled {
        return true;
    }
    // If we've reached the end time, it's effectively terminal even if not yet withdrawn/marked.
    env.ledger().timestamp() >= stream.end_time
}

fn remove_stream(env: &Env, stream_id: u64) {
    let key = DataKey::Stream(stream_id);
    env.storage().persistent().remove(&key);
}

// ---------------------------------------------------------------------------
// Recipient stream index helpers
// ---------------------------------------------------------------------------

/// Load the list of stream IDs for a recipient (sorted by stream_id).
fn load_recipient_streams(env: &Env, recipient: &Address) -> soroban_sdk::Vec<u64> {
    let key = DataKey::RecipientStreams(recipient.clone());
    let streams: soroban_sdk::Vec<u64> = env
        .storage()
        .persistent()
        .get(&key)
        .unwrap_or_else(|| soroban_sdk::Vec::new(env));

    // Only bump TTL if the key exists (has streams)
    if !streams.is_empty() {
        env.storage().persistent().extend_ttl(
            &key,
            PERSISTENT_LIFETIME_THRESHOLD,
            PERSISTENT_BUMP_AMOUNT,
        );
    }

    streams
}

/// Save the list of stream IDs for a recipient (maintains sorted order).
fn save_recipient_streams(env: &Env, recipient: &Address, streams: &soroban_sdk::Vec<u64>) {
    let key = DataKey::RecipientStreams(recipient.clone());
    env.storage().persistent().set(&key, streams);

    // Extend TTL on write to ensure persistence
    env.storage().persistent().extend_ttl(
        &key,
        PERSISTENT_LIFETIME_THRESHOLD,
        PERSISTENT_BUMP_AMOUNT,
    );
}

/// Add a stream ID to a recipient's index (maintains sorted order).
/// Assumes stream_id is not already in the list.
fn add_stream_to_recipient_index(env: &Env, recipient: &Address, stream_id: u64) {
    let mut streams = load_recipient_streams(env, recipient);

    // Insert in sorted order (binary search for insertion point)
    let insert_pos = match streams.binary_search(stream_id) {
        Ok(pos) => pos,
        Err(pos) => pos,
    };

    streams.insert(insert_pos, stream_id);
    save_recipient_streams(env, recipient, &streams);
}

/// Remove a stream ID from a recipient's index.
fn remove_stream_from_recipient_index(env: &Env, recipient: &Address, stream_id: u64) {
    let mut streams = load_recipient_streams(env, recipient);

    // Find and remove the stream_id
    if let Ok(idx) = streams.binary_search(stream_id) {
        streams.remove(idx);
        save_recipient_streams(env, recipient, &streams);
    }
}

// ---------------------------------------------------------------------------
// Liability tracking (total escrow owed to recipients)
// ---------------------------------------------------------------------------

fn read_total_liabilities(env: &Env) -> i128 {
    bump_instance_ttl(env);
    env.storage()
        .instance()
        .get(&DataKey::TotalLiabilities)
        .unwrap_or(0i128)
}

fn write_total_liabilities(env: &Env, amount: i128) {
    env.storage()
        .instance()
        .set(&DataKey::TotalLiabilities, &amount);
    bump_instance_ttl(env);
}

// ---------------------------------------------------------------------------
// Schedule template registry
// ---------------------------------------------------------------------------

fn read_next_template_id(env: &Env) -> u64 {
    bump_instance_ttl(env);
    env.storage()
        .instance()
        .get(&DataKey::NextTemplateId)
        .unwrap_or(0u64)
}

fn set_next_template_id(env: &Env, id: u64) {
    env.storage().instance().set(&DataKey::NextTemplateId, &id);
    bump_instance_ttl(env);
}

fn read_active_template_count(env: &Env) -> u64 {
    bump_instance_ttl(env);
    env.storage()
        .instance()
        .get(&DataKey::ActiveTemplateCount)
        .unwrap_or(0u64)
}

fn set_active_template_count(env: &Env, count: u64) {
    env.storage()
        .instance()
        .set(&DataKey::ActiveTemplateCount, &count);
    bump_instance_ttl(env);
}

fn validate_template_delays(
    env: &Env,
    start_delay: u64,
    cliff_delay: u64,
    duration: u64,
) -> Result<(), ContractError> {
    if duration == 0 {
        return Err(ContractError::InvalidParams);
    }
    if cliff_delay < start_delay {
        return Err(ContractError::InvalidParams);
    }
    let current = env.ledger().timestamp();
    let start_time = current
        .checked_add(start_delay)
        .ok_or(ContractError::InvalidParams)?;
    let cliff_time = current
        .checked_add(cliff_delay)
        .ok_or(ContractError::InvalidParams)?;
    let end_time = start_time
        .checked_add(duration)
        .ok_or(ContractError::InvalidParams)?;
    if cliff_time > end_time {
        return Err(ContractError::InvalidParams);
    }
    Ok(())
}

fn load_owner_template_ids(env: &Env, owner: &Address) -> soroban_sdk::Vec<u64> {
    let key = DataKey::OwnerTemplateIds(owner.clone());
    let ids: soroban_sdk::Vec<u64> = env
        .storage()
        .persistent()
        .get(&key)
        .unwrap_or_else(|| soroban_sdk::Vec::new(env));
    if !ids.is_empty() {
        env.storage().persistent().extend_ttl(
            &key,
            PERSISTENT_LIFETIME_THRESHOLD,
            PERSISTENT_BUMP_AMOUNT,
        );
    }
    ids
}

fn save_owner_template_ids(env: &Env, owner: &Address, ids: &soroban_sdk::Vec<u64>) {
    let key = DataKey::OwnerTemplateIds(owner.clone());
    env.storage().persistent().set(&key, ids);
    env.storage().persistent().extend_ttl(
        &key,
        PERSISTENT_LIFETIME_THRESHOLD,
        PERSISTENT_BUMP_AMOUNT,
    );
}

fn save_stream_template(env: &Env, tpl: &StreamScheduleTemplate) {
    let key = DataKey::StreamTemplate(tpl.template_id);
    env.storage().persistent().set(&key, tpl);
    env.storage().persistent().extend_ttl(
        &key,
        PERSISTENT_LIFETIME_THRESHOLD,
        PERSISTENT_BUMP_AMOUNT,
    );
}

fn load_stream_template(
    env: &Env,
    template_id: u64,
) -> Result<StreamScheduleTemplate, ContractError> {
    let key = DataKey::StreamTemplate(template_id);
    let tpl: StreamScheduleTemplate = env
        .storage()
        .persistent()
        .get(&key)
        .ok_or(ContractError::TemplateNotFound)?;
    env.storage().persistent().extend_ttl(
        &key,
        PERSISTENT_LIFETIME_THRESHOLD,
        PERSISTENT_BUMP_AMOUNT,
    );
    Ok(tpl)
}

fn remove_stream_template_storage(env: &Env, template_id: u64) {
    let key = DataKey::StreamTemplate(template_id);
    env.storage().persistent().remove(&key);
}

fn remove_template_id_for_owner(
    env: &Env,
    owner: &Address,
    template_id: u64,
) -> Result<(), ContractError> {
    let mut ids = load_owner_template_ids(env, owner);
    match ids.binary_search(template_id) {
        Ok(idx) => {
            ids.remove(idx);
            save_owner_template_ids(env, owner, &ids);
            Ok(())
        }
        Err(_) => Err(ContractError::TemplateNotFound),
    }
}

// ---------------------------------------------------------------------------
// Token transfer helpers
// ---------------------------------------------------------------------------
///
/// Centralizes all token transfers INTO the contract for security review.
/// Used when creating streams to pull deposit from sender.
///
/// # Token Trust Model
///
/// This function assumes the token contract is a well-behaved SEP-41 / SAC token that:
/// - Does not re-enter the streaming contract during `transfer`
/// - Does not silently fail (panics or returns an error on insufficient balance)
/// - Implements the standard Soroban token interface
///
/// If a malicious token violates these assumptions, the CEI pattern reduces but does not
/// eliminate reentrancy impact — state will already reflect the current operation when
/// the re-entry occurs.
///
/// # Parameters
/// - `env`: Contract environment
/// - `from`: Address to transfer tokens from (must have approved contract)
/// - `amount`: Amount of tokens to transfer
///
/// # Panics
/// - If token transfer fails (insufficient balance or allowance)
/// - If token contract panics or returns an error
///
/// # Security Notes
/// - CEI ordering: State is persisted BEFORE calling this function to reduce reentrancy risk
/// - Atomic transaction: If this function panics, the entire transaction reverts
/// - No silent failures: Token transfer either succeeds or fails explicitly
///
/// See [`token-assumptions.md`](../../docs/token-assumptions.md) for complete token trust model.
fn pull_token(env: &Env, from: &Address, amount: i128) -> Result<(), ContractError> {
    let token_address = get_token(env)?;
    let token_client = token::Client::new(env, &token_address);
    token_client.transfer_from(
        &env.current_contract_address(),
        from,
        &env.current_contract_address(),
        &amount,
    );
    Ok(())
}

/// Push tokens from the contract to an external address.
///
/// Centralizes all token transfers OUT OF the contract for security review.
/// Used for withdrawals (to recipient) and refunds (to sender on cancel).
///
/// # Token Trust Model
///
/// This function assumes the token contract is a well-behaved SEP-41 / SAC token that:
/// - Does not re-enter the streaming contract during `transfer`
/// - Does not silently fail (panics or returns an error on insufficient balance)
/// - Implements the standard Soroban token interface
///
/// If a malicious token violates these assumptions, the CEI pattern reduces but does not
/// eliminate reentrancy impact — state will already reflect the current operation when
/// the re-entry occurs.
///
/// # Parameters
/// - `env`: Contract environment
/// - `to`: Address to transfer tokens to
/// - `amount`: Amount of tokens to transfer
///
/// # Panics
/// - If token transfer fails (insufficient contract balance, should not happen)
/// - If token contract panics or returns an error
///
/// # Security Notes
/// - CEI ordering: State is persisted BEFORE calling this function to reduce reentrancy risk
/// - Atomic transaction: If this function panics, the entire transaction reverts
/// - No silent failures: Token transfer either succeeds or fails explicitly
///
/// See [`token-assumptions.md`](../../docs/token-assumptions.md) for complete token trust model.
fn push_token(env: &Env, to: &Address, amount: i128) -> Result<(), ContractError> {
    let token_address = get_token(env)?;
    let token_client = token::Client::new(env, &token_address);
    token_client.transfer(&env.current_contract_address(), to, &amount);
    Ok(())
}

// ---------------------------------------------------------------------------
// Internal Helpers
// ---------------------------------------------------------------------------

impl FluxoraStream {
    #[allow(clippy::too_many_arguments)]
    fn validate_stream_params(
        _env: &Env,
        sender: &Address,
        recipient: &Address,
        deposit_amount: i128,
        rate_per_second: i128,
        current_ledger_timestamp: u64,
        start_time: u64,
        cliff_time: u64,
        end_time: u64,
    ) -> Result<(), ContractError> {
        // Validate positive amounts (#35)
        if deposit_amount <= 0 || rate_per_second <= 0 {
            return Err(ContractError::InvalidParams);
        }

        // Validate sender != recipient (#35)
        if sender == recipient {
            return Err(ContractError::InvalidParams);
        }

        // Validate time constraints
        if start_time >= end_time {
            return Err(ContractError::InvalidParams);
        }
        if start_time < current_ledger_timestamp {
            return Err(ContractError::StartTimeInPast);
        }
        if cliff_time < start_time || cliff_time > end_time {
            return Err(ContractError::InvalidParams);
        }

        // Validate deposit covers total streamable amount (#34)
        let duration = (end_time - start_time) as i128;
        let total_streamable = rate_per_second
            .checked_mul(duration)
            .ok_or(ContractError::InvalidParams)?; // Return InvalidParams on overflow as expected by tests

        if deposit_amount < total_streamable {
            return Err(ContractError::InsufficientDeposit);
        }

        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn persist_new_stream(
        env: &Env,
        sender: Address,
        recipient: Address,
        deposit_amount: i128,
        rate_per_second: i128,
        start_time: u64,
        cliff_time: u64,
        end_time: u64,
        withdraw_dust_threshold: i128,
        memo: Option<soroban_sdk::Bytes>,
    ) -> Result<u64, ContractError> {
        // Validate memo length before allocating a stream ID.
        if let Some(ref m) = memo {
            if m.len() as usize > MAX_MEMO_BYTES {
                return Err(ContractError::InvalidParams);
            }
        }

        let stream_id = read_stream_count(env);
        set_stream_count(env, stream_id + 1);

        let stream = Stream {
            stream_id,
            sender: sender.clone(),
            recipient: recipient.clone(),
            deposit_amount,
            rate_per_second,
            start_time,
            cliff_time,
            end_time,
            withdrawn_amount: 0,
            status: StreamStatus::Active,
            cancelled_at: None,
            checkpointed_amount: 0,
            checkpointed_at: start_time,
            withdraw_dust_threshold,
            memo: memo.clone(),
        };

        save_stream(env, &stream);

        // Add stream to recipient's index (maintains sorted order by stream_id)
        add_stream_to_recipient_index(env, &recipient, stream_id);

        // Track liability: the full deposit is owed to the recipient until withdrawn/refunded.
        let liabilities = read_total_liabilities(env)
            .checked_add(deposit_amount)
            .unwrap_or(i128::MAX);
        write_total_liabilities(env, liabilities);

        env.events().publish(
            (symbol_short!("created"), stream_id),
            StreamCreated {
                stream_id,
                sender,
                recipient,
                deposit_amount,
                rate_per_second,
                start_time,
                cliff_time,
                end_time,
                withdraw_dust_threshold,
                memo,
            },
        );

        Ok(stream_id)
    }
}

// ---------------------------------------------------------------------------
// Contract Implementation
// ---------------------------------------------------------------------------

#[contract]
pub struct FluxoraStream;

#[allow(clippy::too_many_arguments)]
#[contractimpl]
impl FluxoraStream {
    /// Initialise the contract with the streaming token and admin address.
    ///
    /// This function must be called exactly once before any other contract operations.
    /// It persists the token address (used for all stream transfers) and admin address
    /// (authorized for administrative operations) in instance storage.
    ///
    /// # Parameters
    /// - `token`: Address of the token contract used for all payment streams
    /// - `admin`: Address authorized to perform administrative operations (pause, cancel, etc.)
    ///   and required to authorize this bootstrap transaction
    ///
    /// # Storage
    /// - Stores `Config { token, admin }` in instance storage under `DataKey::Config`
    /// - Initializes `NextStreamId` counter to 0 for stream ID generation
    /// - Extends TTL to prevent premature expiration (17280 ledgers threshold, 120960 max)
    ///
    /// # Panics
    /// - If called more than once (contract already initialized)
    /// - If `admin` does not authorize the call
    ///
    /// # Security
    /// - Bootstrap authorization is explicit: only a signer controlling `admin` can initialize
    /// - Re-initialization is prevented to ensure immutable token and admin configuration
    /// - Failed re-initialization attempts are side-effect free (config/counter unchanged)
    ///
    /// # Token Trust Model
    ///
    /// The `token` address is stored immutably after initialization. All subsequent token
    /// operations (transfers) will use this address. The contract assumes the token at this
    /// address is a well-behaved SEP-41 / SAC token that:
    /// - Does not re-enter the streaming contract during transfers
    /// - Does not silently fail (panics or returns an error on insufficient balance)
    /// - Implements the standard Soroban token interface
    ///
    /// **Operators are responsible for verifying token behavior before initialization.**
    /// If a malicious token is used, the contract's behavior may become unpredictable.
    ///
    /// See [`token-assumptions.md`](../../docs/token-assumptions.md) for complete token trust model.
    pub fn init(env: Env, token: Address, admin: Address) -> Result<(), ContractError> {
        admin.require_auth();
        if env.storage().instance().has(&DataKey::Config) {
            return Err(ContractError::AlreadyInitialised);
        }
        let config = Config { token, admin };
        env.storage().instance().set(&DataKey::Config, &config);
        env.storage().instance().set(&DataKey::NextStreamId, &0u64);
        env.storage()
            .instance()
            .set(&DataKey::NextTemplateId, &0u64);
        env.storage()
            .instance()
            .set(&DataKey::ActiveTemplateCount, &0u64);
        env.storage()
            .instance()
            .set(&DataKey::TotalLiabilities, &0i128);

        // Ensure instance storage (Config / NextStreamId) doesn't expire quickly
        bump_instance_ttl(&env);
        Ok(())
    }

    /// Create a new payment stream with specified parameters.
    ///
    /// Establishes a new token stream from sender to recipient with defined rate and duration.
    /// Transfers the deposit amount from sender to the contract immediately. Returns a unique
    /// stream ID that can be used to interact with the stream.
    ///
    /// # Parameters
    /// - `sender`: Address funding the stream (must authorize the transaction)
    /// - `recipient`: Address receiving the streamed tokens
    /// - `deposit_amount`: Total tokens to deposit (must be > 0 and <= i128::MAX)
    /// - `rate_per_second`: Streaming rate in tokens per second (must be > 0)
    /// - `start_time`: When streaming begins (ledger timestamp)
    /// - `cliff_time`: When tokens first become available (vesting cliff, must be in [start_time, end_time])
    /// - `end_time`: When streaming completes (must be > start_time)
    ///
    /// # Returns
    /// - `u64`: Unique stream identifier for the newly created stream
    ///
    /// # Authorization
    /// - Requires authorization from the sender address
    ///
    /// # Validation
    /// The function validates all parameters before creating the stream:
    /// - `deposit_amount > 0` and `rate_per_second > 0`
    /// - `sender != recipient` (cannot stream to yourself)
    /// - `start_time < end_time` (valid time range)
    /// - `start_time >= ledger timestamp` (start_time must not be in the past)
    /// - `cliff_time` in `[start_time, end_time]` (cliff within stream duration)
    /// - `deposit_amount >= rate_per_second × (end_time - start_time)` (sufficient deposit)
    ///
    /// # Panics
    /// - If `start_time` is before the current ledger timestamp (past start time)
    ///   - Uses `ContractError::StartTimeInPast` (structured error for integrators)
    /// - If `deposit_amount` or `rate_per_second` is not positive
    /// - If `sender` and `recipient` are the same address
    /// - If `start_time >= end_time` (invalid time range)
    /// - If `cliff_time` is not in `[start_time, end_time]`
    /// - If `deposit_amount < rate_per_second × (end_time - start_time)` (insufficient deposit)
    /// - If token transfer fails (insufficient balance or allowance)
    /// - If overflow occurs calculating total streamable amount
    ///
    /// # State Changes
    /// - Transfers `deposit_amount` tokens from sender to contract
    /// - Creates new stream with status `Active`
    /// - Increments global stream counter
    /// - Stores stream data in persistent storage with extended TTL
    ///
    /// # Events
    /// - Publishes `created(stream_id, deposit_amount)` event on success
    ///
    /// # Usage Notes
    /// - Self-streaming is disallowed: `sender` must be different from `recipient`
    ///   - Violations panic with `"sender and recipient must be different"`
    ///   - No state is persisted, no tokens move, and no `created` event is emitted
    /// - Transaction is atomic: if token transfer fails, no stream is created
    /// - Stream IDs are sequential starting from 0
    /// - Cliff time enables vesting schedules (no withdrawals before cliff)
    /// - Setting `cliff_time = start_time` means no cliff (immediate vesting)
    /// - Deposit can exceed minimum required (excess remains in contract)
    /// - Sender must have sufficient token balance and approve contract
    /// ## Stream Limits Policy
    /// No hard upper bounds (e.g. "max 1 million tokens") are enforced on `deposit_amount`
    /// beyond the technical limit of `i128::MAX` and the underlying token's supply.
    /// Rationale:
    /// - Overflow in accrual math is already prevented via `checked_mul` and clamping (Issue #6).
    /// - A fixed arbitrary cap would require a contract upgrade to change and conflicts with
    ///   the overflow test suite, which exercises values up to `i128::MAX`.
    /// - Protocol-specific or business-driven limits belong at the application layer.
    /// - This contract remains "defense in depth" by ensuring math safety at all scales.
    ///
    /// Senders are responsible for the correctness of the values they supply.
    /// The validations above (`deposit > 0`, `rate > 0`, `deposit >= rate × duration`,
    /// valid time window) are the contract's complete set of creation constraints.
    ///
    ///
    /// # Errors
    /// Returns `ContractError` if:
    /// - `ContractPaused` (4): Operations are globally halted; new streams cannot be created.
    /// - `InvalidParams` (3): Negative values, zero durations, or insufficient starting deposit.
    /// - `StartTimeInPast` (5): The `start_time` is strictly before the current ledger timestamp.
    /// - `ArithmeticOverflow` (6): Value conversions or deposit sum exceeds safe capacities.
    /// - `Unauthorized` (7): Sender signature is missing.
    ///
    /// # Examples
    /// - Linear stream: 1000 tokens over 1000 seconds, no cliff
    ///   - `deposit_amount = 1000`, `rate = 1`, `start = 0`, `cliff = 0`, `end = 1000`
    /// - Vesting stream: 12000 tokens over 12 months, 6-month cliff
    ///   - `deposit_amount = 12000`, `rate = 1`, `start = 0`, `cliff = 15552000`, `end = 31104000`
    #[allow(clippy::too_many_arguments)]
    pub fn create_stream(
        env: Env,
        sender: Address,
        recipient: Address,
        deposit_amount: i128,
        rate_per_second: i128,
        start_time: u64,
        cliff_time: u64,
        end_time: u64,
        withdraw_dust_threshold: i128,
        memo: Option<soroban_sdk::Bytes>,
    ) -> Result<u64, ContractError> {
        sender.require_auth();
        require_not_creation_paused(&env)?;

        Self::validate_stream_params(
            &env,
            &sender,
            &recipient,
            deposit_amount,
            rate_per_second,
            env.ledger().timestamp(),
            start_time,
            cliff_time,
            end_time,
        )?;

        pull_token(&env, &sender, deposit_amount)?;

        Self::persist_new_stream(
            &env,
            sender,
            recipient,
            deposit_amount,
            rate_per_second,
            start_time,
            cliff_time,
            end_time,
            withdraw_dust_threshold,
            memo,
        )
    }

    /// Create a new payment stream with relative (offset-based) timing.
    ///
    /// Computes absolute timestamps by adding delays to the current ledger timestamp,
    /// eliminating off-chain calculation errors that cause `StartTimeInPast` failures.
    /// Internally delegates to `create_stream` with computed absolute times.
    ///
    /// # Parameters
    /// - `sender`: Address funding and authorizing the stream
    /// - `recipient`: Address receiving streamed tokens
    /// - `deposit_amount`: Total amount escrowed for the stream
    /// - `rate_per_second`: Streaming speed in tokens per second
    /// - `start_delay`: Seconds until stream starts (relative to current timestamp)
    /// - `cliff_delay`: Seconds until cliff time (relative to current timestamp)
    /// - `duration`: Total duration stream runs (in seconds)
    ///
    /// # Computation
    /// Uses `current_time = env.ledger().timestamp()`:
    /// - `start_time = current_time + start_delay`
    /// - `cliff_time = current_time + cliff_delay`
    /// - `end_time = start_time + duration`
    ///
    /// # Returns
    /// - `u64`: Unique stream ID
    ///
    /// # Authorization
    /// - Requires authorization from `sender`
    ///
    /// # Success Semantics
    /// - All validation invariants from `create_stream` are preserved
    /// - Batch `create_streams_relative` can use this via parameter conversion
    /// - Contract paused state is checked (blocks creation if paused)
    ///
    /// # Failure Semantics
    /// - `StartTimeInPast`: Never occurs (times are always relative to current)
    /// - `InvalidParams`: If delays/duration cause arithmetic overflow or invalid constraints
    /// - `ContractPaused`: If creation is globally paused
    /// - Other errors inherited from `create_stream` validation
    ///
    /// # Errors
    /// Delegates to `create_stream`; see its documentation for full error list.
    ///
    /// # Panics
    /// - If `start_delay + current_time` overflows `u64` (arithmetic overflow)
    /// - If token transfer fails
    ///
    /// # Security Notes
    /// - Relative timing removes the need for precise off-chain clock synchronization
    /// - All deposit and rate validation proceeds as-is; relative delays do not bypass checks
    /// - Self-streaming (`sender == recipient`) is still rejected by `create_stream`
    ///
    /// # Example
    /// ```
    /// // Create a stream starting in 2 days, cliff in 5 days, running for 30 days
    /// let stream_id = contract.create_stream_relative(
    ///     &sender,
    ///     &recipient,
    ///     &100_000_000,        // 100M tokens
    ///     &1_157_407,           // ~1% per day at 86400s/day
    ///     &(2 * 86400),         // 2 days delay
    ///     &(5 * 86400),         // 5 days cliff
    ///     &(30 * 86400),        // 30 days duration
    /// )?;
    /// ```
    #[allow(clippy::too_many_arguments)]
    pub fn create_stream_relative(
        env: Env,
        sender: Address,
        params: CreateStreamRelativeParams,
    ) -> Result<u64, ContractError> {
        Self::create_stream_relative_inner(env, sender, params)
    }

    fn create_stream_relative_inner(
        env: Env,
        sender: Address,
        params: CreateStreamRelativeParams,
    ) -> Result<u64, ContractError> {
        let current_time = env.ledger().timestamp();

        // Compute absolute times with overflow checks
        let start_time = current_time
            .checked_add(params.start_delay)
            .ok_or(ContractError::InvalidParams)?;
        let cliff_time = current_time
            .checked_add(params.cliff_delay)
            .ok_or(ContractError::InvalidParams)?;
        let end_time = start_time
            .checked_add(params.duration)
            .ok_or(ContractError::InvalidParams)?;

        // Delegate to standard create_stream with computed absolute times
        Self::create_stream(
            env,
            sender,
            params.recipient,
            params.deposit_amount,
            params.rate_per_second,
            start_time,
            cliff_time,
            end_time,
            params.withdraw_dust_threshold.unwrap_or(0),
            params.memo,
        )
    }

    /// Create multiple payment streams in a single transaction.
    ///
    /// Optimizes gas usage by authorizing once and doing a single bulk token transfer
    /// for all streams. The batch is atomic: either all streams are created, or none are.
    ///
    /// # Parameters
    /// - `sender`: Address funding all streams in the batch
    /// - `streams`: Vector of stream configuration parameters
    ///
    /// # Returns
    /// - `Vec<u64>`: Stream IDs in the same order as `streams` input entries
    ///
    /// # Authorization
    /// - Requires authorization from `sender` exactly once for the entire batch
    ///
    /// # Success Semantics
    /// - Every entry is validated using the same rules as `create_stream`
    /// - The total deposit is computed as `sum(entry.deposit_amount)` with checked arithmetic
    /// - A single token transfer pulls the total from `sender` into the contract
    /// - Streams are persisted sequentially with contiguous IDs and one `created` event per stream
    ///
    /// # Failure Semantics
    /// - Any validation failure, arithmetic overflow, auth failure, or token transfer failure aborts the call
    /// - On failure there are no persistent writes, no token movement, and no `created` events
    /// - If the contract is globally paused (`ContractPaused`), the entire batch is rejected
    ///
    /// # Errors
    /// Returns `ContractError` if:
    /// - `ContractPaused` (4): Operations are globally halted; batch creation is completely blocked.
    /// - `InvalidParams` (3): An entry contains negative values, zero durations, etc.
    /// - `StartTimeInPast` (5): An entry's `start_time` is before the current ledger timestamp.
    /// - `ArithmeticOverflow` (6): Value conversions or total batch deposit exceeds `i128::MAX`.
    /// - `Unauthorized` (7): Sender signature is missing.
    ///
    /// # Panics
    /// - If token transfer fails due to sender balance/allowance constraints
    ///
    /// # Security Notes
    /// - Self-streaming is disallowed per entry: `sender` must not equal `recipient`
    /// - Validation is completed before any external token interaction
    /// Create multiple payment streams in a single atomic batch operation.
    ///
    /// This function enables treasury operators and integrators to create multiple streams
    /// with a single authorization and token transfer, reducing gas costs and ensuring
    /// all-or-nothing semantics.
    ///
    /// # Parameters
    /// - `sender`: Address that funds and authorizes the batch (must authorize this call)
    /// - `streams`: Vector of `CreateStreamParams` defining each stream's schedule and recipient
    ///
    /// # Authorization
    /// - Requires `sender.require_auth()` (single auth check for entire batch)
    /// - Fails atomically if sender is not authorized
    ///
    /// # Empty Vector Semantics
    /// When `streams` is empty:
    /// - Returns `Ok(Vec::new())` (empty result vector)
    /// - No tokens are transferred (total_deposit = 0)
    /// - No streams are persisted
    /// - No `StreamCreated` events are emitted
    /// - Stream ID counter is not advanced
    /// - Authorization is still required (sender must authorize the call)
    /// - Contract state remains unchanged
    /// - No errors are raised (empty batch is valid)
    ///
    /// # Success Semantics
    /// When `streams` is non-empty:
    /// 1. All entries are validated before any state changes (first pass)
    /// 2. Total deposit is calculated with overflow protection
    /// 3. Tokens are transferred atomically: `sum(deposit_amount)` from sender to contract
    /// 4. Stream IDs are allocated sequentially (contiguous, starting from next available ID)
    /// 5. Each stream is persisted with status `Active`
    /// 6. Recipient stream index is updated (sorted by stream_id)
    /// 7. One `StreamCreated` event is emitted per stream (in order)
    /// 8. Returned vector contains stream IDs in the same order as input entries
    ///
    /// # Failure Semantics
    /// If any validation fails (or total-deposit sum overflows):
    /// - No streams are created
    /// - No tokens are transferred
    /// - No events are emitted
    /// - Stream ID counter is not advanced
    /// - Entire batch is reverted (atomic)
    /// - Error is returned to caller
    ///
    /// Validation failures include:
    /// - Any entry has invalid parameters (see `validate_stream_params`)
    /// - Total deposit sum overflows `i128`
    /// - Contract is globally paused
    /// - Sender is not authorized
    ///
    /// # Invariants After Success
    /// - `returned_ids.len() == streams.len()`
    /// - `returned_ids[i]` is the ID of the stream created from `streams[i]`
    /// - Each stream has status `Active` and `withdrawn_amount = 0`
    /// - Each recipient's stream index includes the new stream_id (sorted)
    /// - Total tokens transferred = `sum(deposit_amount)`
    /// - Stream ID counter advanced by `streams.len()`
    ///
    /// # Gas Considerations
    /// - Single token transfer (vs. N transfers for N individual `create_stream` calls)
    /// - Batch validation reduces redundant checks
    /// - Recipient index updates are O(n log n) total (binary search per stream)
    ///
    /// # Events
    /// - On success: one `StreamCreated` event per stream
    /// - On failure: no events
    /// - On empty batch: no events
    ///
    /// # Example
    /// ```ignore
    /// let params = vec![
    ///     CreateStreamParams { recipient: alice, deposit_amount: 1000, ... },
    ///     CreateStreamParams { recipient: bob, deposit_amount: 2000, ... },
    /// ];
    /// let ids = contract.create_streams(&sender, &params)?;
    /// // ids = [0, 1] (assuming first batch)
    /// ```
    pub fn create_streams(
        env: Env,
        sender: Address,
        streams: soroban_sdk::Vec<CreateStreamParams>,
    ) -> Result<soroban_sdk::Vec<u64>, ContractError> {
        sender.require_auth();

        if streams.is_empty() {
            return Ok(soroban_sdk::Vec::new(&env));
        }

        require_not_creation_paused(&env)?;

        let current_time = env.ledger().timestamp();
        let mut total_deposit: i128 = 0;

        // First pass: validate all streams and calculate total deposit required
        for params in streams.iter() {
            Self::validate_stream_params(
                &env,
                &sender,
                &params.recipient,
                params.deposit_amount,
                params.rate_per_second,
                current_time,
                params.start_time,
                params.cliff_time,
                params.end_time,
            )?;
            total_deposit = total_deposit
                .checked_add(params.deposit_amount)
                .ok_or(ContractError::ArithmeticOverflow)?;
        }

        // Bulk transfer tokens from sender to this contract atomically to save gas.
        // Empty batch: total_deposit = 0, no transfer occurs.
        if total_deposit > 0 {
            pull_token(&env, &sender, total_deposit)?;
        }

        // Second pass: generate IDs, persist state, and emit events iteratively
        let mut created_ids = soroban_sdk::Vec::new(&env);
        for params in streams.iter() {
            let stream_id = Self::persist_new_stream(
                &env,
                sender.clone(),
                params.recipient,
                params.deposit_amount,
                params.rate_per_second,
                params.start_time,
                params.cliff_time,
                params.end_time,
                params.withdraw_dust_threshold.unwrap_or(0),
                params.memo,
            )?;
            created_ids.push_back(stream_id);
        }

        Ok(created_ids)
    }

    /// Create multiple payment streams with relative (offset-based) timing.
    ///
    /// Batch version of `create_stream_relative` that converts relative time offsets
    /// to absolute timestamps and delegates to `create_streams`. Provides the same
    /// atomicity and gas efficiency as `create_streams` while eliminating off-chain
    /// timestamp calculation errors.
    ///
    /// # Parameters
    /// - `sender`: Address funding all streams in the batch
    /// - `streams_relative`: Vector of `CreateStreamRelativeParams` with relative time offsets
    ///
    /// # Returns
    /// - `Vec<u64>`: Stream IDs in the same order as `streams_relative` input entries
    ///
    /// # Authorization
    /// - Requires authorization from `sender` exactly once for the entire batch
    ///
    /// # Time Computation
    /// Uses `current_time = env.ledger().timestamp()`:
    /// For each entry:
    /// - `start_time = current_time + start_delay`
    /// - `cliff_time = current_time + cliff_delay`
    /// - `end_time = start_time + duration`
    ///
    /// # Success Semantics
    /// - Empty batch returns `Ok(Vec::new())` without side effects
    /// - All validation invariants from `create_streams` are preserved
    /// - Relative timing eliminates `StartTimeInPast` errors
    /// - Single token transfer for all streams (atomic)
    ///
    /// # Failure Semantics
    /// - Any entry's time offset causes arithmetic overflow → `InvalidParams`
    /// - Any validation failure → entire batch fails atomically
    /// - Any token transfer failure → no state change
    /// - No events emitted on failure
    ///
    /// # Security Notes
    /// - Relative timing removes need for off-chain clock synchronization
    /// - All deposit, rate, and deposit-coverage validation proceeds as-is
    /// - Self-streaming still rejected per entry
    ///
    /// # Example
    /// ```ignore
    /// let params = vec![
    ///     CreateStreamRelativeParams {
    ///         recipient: alice,
    ///         deposit_amount: 1000,
    ///         rate_per_second: 1,
    ///         start_delay: 86400,      // 1 day
    ///         cliff_delay: 259200,     // 3 days
    ///         duration: 2592000,       // 30 days
    ///         withdraw_dust_threshold: 0,
    ///     },
    ///     CreateStreamRelativeParams {
    ///         recipient: bob,
    ///         deposit_amount: 2000,
    ///         rate_per_second: 2,
    ///         start_delay: 0,          // Immediate
    ///         cliff_delay: 0,          // Immediate
    ///         duration: 2592000,       // 30 days
    ///         withdraw_dust_threshold: 0,
    ///     },
    /// ];
    /// let ids = contract.create_streams_relative(&sender, &params)?;
    /// // ids = [0, 1] (assuming first batch)
    /// ```
    pub fn create_streams_relative(
        env: Env,
        sender: Address,
        streams_relative: soroban_sdk::Vec<CreateStreamRelativeParams>,
    ) -> Result<soroban_sdk::Vec<u64>, ContractError> {
        if streams_relative.is_empty() {
            return Ok(soroban_sdk::Vec::new(&env));
        }

        let current_time = env.ledger().timestamp();
        let mut absolute_streams = soroban_sdk::Vec::new(&env);

        // Convert relative parameters to absolute times
        for rel in streams_relative.iter() {
            let start_time = current_time
                .checked_add(rel.start_delay)
                .ok_or(ContractError::InvalidParams)?;
            let cliff_time = current_time
                .checked_add(rel.cliff_delay)
                .ok_or(ContractError::InvalidParams)?;
            let end_time = start_time
                .checked_add(rel.duration)
                .ok_or(ContractError::InvalidParams)?;

            absolute_streams.push_back(CreateStreamParams {
                recipient: rel.recipient,
                deposit_amount: rel.deposit_amount,
                rate_per_second: rel.rate_per_second,
                start_time,
                cliff_time,
                end_time,
                withdraw_dust_threshold: rel.withdraw_dust_threshold,
                memo: rel.memo,
            });
        }

        // Delegate to standard create_streams with converted absolute times
        Self::create_streams(env, sender, absolute_streams)
    }

    /// Pause an active payment stream.
    ///
    /// Temporarily halts withdrawals from the stream while preserving accrual calculations.
    /// The stream can be resumed later by the sender or admin. Accrual continues based on
    /// time elapsed, but the recipient cannot withdraw while paused.
    ///
    /// # Parameters
    /// - `stream_id`: Unique identifier of the stream to pause
    /// - `reason`: Operational reason code for the pause (see `PauseReason`)
    ///
    /// # Authorization
    /// - Requires authorization from the stream's sender (original creator)
    /// - Admin can use `pause_stream_as_admin` for administrative override
    ///
    /// # Panics
    /// - If the stream is not in `Active` state (already paused, completed, or cancelled)
    /// - If the stream does not exist (`stream_id` is invalid)
    /// - If caller is not authorized (not the sender)
    ///
    /// # Events
    /// - Publishes `("paused", stream_id)` → `StreamPaused { stream_id, reason }` on success
    ///
    /// # Usage Notes
    /// - Pausing does not affect accrual calculations (time-based)
    /// - Recipient cannot withdraw while stream is paused
    /// - Stream can be cancelled while paused
    /// - Use `resume_stream` to reactivate withdrawals
    pub fn pause_stream(
        env: Env,
        stream_id: u64,
        reason: PauseReason,
    ) -> Result<(), ContractError> {
        let mut stream = load_stream(&env, stream_id)?;

        Self::require_stream_sender(&stream.sender);

        if stream.status == StreamStatus::Paused {
            return Err(ContractError::StreamAlreadyPaused);
        }

        if is_terminal_state(&env, &stream) {
            return Err(ContractError::StreamTerminalState);
        }

        if stream.status != StreamStatus::Active {
            return Err(ContractError::InvalidState);
        }

        stream.status = StreamStatus::Paused;
        save_stream(&env, &stream);

        env.events().publish(
            (symbol_short!("paused"), stream_id),
            StreamPaused { stream_id, reason },
        );
        Ok(())
    }

    /// Resume a paused payment stream.
    ///
    /// Reactivates a paused stream, allowing the recipient to withdraw accrued funds again.
    /// Only streams in `Paused` state can be resumed. Terminal states (Completed, Cancelled)
    /// cannot be resumed.
    ///
    /// # Parameters
    /// - `stream_id`: Unique identifier of the stream to resume
    ///
    /// # Authorization
    /// - Requires authorization from the stream's sender (original creator)
    /// - Admin can use `resume_stream_as_admin` for administrative override
    ///
    /// # Panics
    /// - If the stream is `Active` (not paused, already running)
    /// - If the stream is `Completed` (terminal state, cannot be resumed)
    /// - If the stream is `Cancelled` (terminal state, cannot be resumed)
    /// - If the stream does not exist (`stream_id` is invalid)
    /// - If caller is not authorized (not the sender)
    ///
    /// # Events
    /// - Publishes `Resumed(stream_id)` event on success
    ///
    /// # Usage Notes
    /// - Only paused streams can be resumed
    /// - Accrual calculations are time-based and unaffected by pause/resume
    /// - After resume, recipient can immediately withdraw accrued funds
    pub fn resume_stream(env: Env, stream_id: u64) -> Result<(), ContractError> {
        let mut stream = load_stream(&env, stream_id)?;
        Self::require_stream_sender(&stream.sender);

        if stream.status == StreamStatus::Active {
            return Err(ContractError::StreamNotPaused);
        }
        if is_terminal_state(&env, &stream) {
            return Err(ContractError::StreamTerminalState);
        }
        if stream.status != StreamStatus::Paused {
            return Err(ContractError::StreamNotPaused);
        }

        stream.status = StreamStatus::Active;
        save_stream(&env, &stream);

        env.events().publish(
            (symbol_short!("resumed"), stream_id),
            StreamEvent::Resumed(stream_id),
        );
        Ok(())
    }

    /// Cancel a payment stream and refund unstreamed funds to the sender.
    ///
    /// Terminates an active or paused stream, immediately refunding any unstreamed tokens
    /// to the sender. The accrued amount (based on time elapsed) remains in the contract
    /// for the recipient to withdraw. This is a terminal operation - cancelled streams
    /// cannot be resumed.
    ///
    /// # Parameters
    /// - `stream_id`: Unique identifier of the stream to cancel
    ///
    /// # Authorization
    /// - Requires authorization from the stream's sender (original creator)
    /// - Admin can use `cancel_stream_as_admin` for administrative override
    ///
    /// # Behavior
    /// 1. Validates stream is in `Active` or `Paused` state
    /// 2. Captures cancellation timestamp: `now = ledger.timestamp()`
    /// 3. Calculates accrued amount at `now`: `min((now - start_time) × rate, deposit_amount)`
    /// 4. Calculates refund: `deposit_amount - accrued_at_now`
    /// 5. Persists terminal state before transfer:
    ///    - `status = Cancelled`
    ///    - `cancelled_at = Some(now)`
    /// 6. Transfers refund to sender (if > 0)
    /// 7. Emits `StreamCancelled(stream_id)` event
    ///
    /// # Returns
    /// - Implicitly returns via state change and token transfer
    ///
    /// # Panics
    /// - Returns `ContractError::InvalidState` if stream is not `Active` or `Paused`
    /// - If the stream does not exist (`stream_id` is invalid)
    /// - If caller is not authorized (not the sender)
    /// - If token transfer fails (should not happen with valid contract state)
    ///
    /// # Events
    /// - Publishes `Cancelled(stream_id)` event on success
    ///
    /// # Usage Notes
    /// - Cancellation is irreversible (terminal state)
    /// - Recipient can still withdraw accrued amount after cancellation
    /// - If fully accrued (time >= end_time), sender receives no refund
    /// - Accrual is time-based, not affected by pause state
    /// - Can be called on paused streams
    ///
    /// # Handling of already-accrued amount
    /// - The accrued portion of the stream (based on time, up to `deposit_amount`)
    ///   is **never** refunded to the sender.
    /// - It remains locked in the contract and can only be claimed by the recipient
    ///   via `withdraw()`.
    /// - The contract does **not** auto-transfer accrued funds to the recipient when
    ///   cancelling; the recipient must explicitly withdraw.
    ///
    /// # Examples
    /// - Cancel at 30% completion → sender gets 70% refund, recipient can withdraw 30%
    /// - Cancel at 100% completion → sender gets 0% refund, recipient can withdraw 100%
    /// - Cancel before cliff → sender gets 100% refund (no accrual before cliff)
    pub fn cancel_stream(env: Env, stream_id: u64) -> Result<(), ContractError> {
        require_not_globally_paused(&env)?;
        let mut stream = load_stream(&env, stream_id)?;
        Self::require_stream_sender(&stream.sender);
        Self::cancel_stream_internal(&env, &mut stream)
    }

    /// Withdraw accrued tokens from a payment stream to the recipient.
    ///
    /// Transfers all accrued-but-not-yet-withdrawn tokens to the stream's recipient.
    /// The amount withdrawn is calculated as `accrued - withdrawn_amount`, where accrued
    /// is based on time elapsed since stream start. If this withdrawal completes the
    /// stream (all deposited tokens withdrawn), the stream status transitions to `Completed`.
    ///
    /// # Parameters
    /// - `stream_id`: Unique identifier of the stream to withdraw from
    ///
    /// # Returns
    /// - `i128`: The amount of tokens transferred to the recipient (0 if nothing to withdraw)
    ///
    /// # Authorization
    /// - Requires authorization from the stream's recipient (only recipient can withdraw)
    /// - This prevents anyone from withdrawing on behalf of the recipient
    ///
    /// # Zero Withdrawable Behavior
    /// - If `accrued == withdrawn_amount` (nothing to withdraw), returns 0 immediately.
    /// - No token transfer occurs, no state is modified or saved, and no events are published.
    /// - This is idempotent: safe to call continuously without state churn or cost footprint.
    /// - Occurs before cliff, after a full claim, or when the stream is already drained to its cancellation point.
    /// - Frontends and indexers can safely poll `withdraw` without pre-checking the balance.
    ///
    /// # Panics
    /// - If the stream is `Completed` (all tokens already withdrawn)
    /// - If the stream is `Paused` (withdrawals not allowed while paused)
    /// - If the stream does not exist (`stream_id` is invalid)
    /// - If caller is not authorized (not the recipient)
    /// - If token transfer fails (insufficient contract balance, should not happen)
    ///
    /// # State Changes
    /// - Updates `withdrawn_amount` by the amount transferred (only if withdrawable > 0)
    /// - Sets status to `Completed` only when withdrawing from an `Active` stream and all
    ///   deposited tokens are withdrawn
    /// - Extends stream storage TTL to prevent expiration
    ///
    /// # Events
    /// - Publishes `withdrew(stream_id, amount)` event on success (only if amount > 0)
    ///
    /// # Usage Notes
    /// - Can be called multiple times to withdraw incrementally
    /// - Accrual is time-based: `min((now - start_time) × rate, deposit_amount)`
    /// - Before cliff time, accrued amount is 0 (returns 0, no transfer)
    /// - After end_time, accrued amount is capped at deposit_amount
    /// - Works on `Active` and `Cancelled` streams, not on `Paused` or `Completed`
    /// - For cancelled streams, only the accrued amount (not refunded) can be withdrawn,
    ///   and status remains `Cancelled` (no `Completed` transition)
    ///
    /// # Examples
    /// - Stream: 1000 tokens over 1000 seconds (1 token/sec)
    /// - At t=0 (before cliff): withdraw() returns 0 (no transfer)
    /// - At t=300: withdraw() returns 300 tokens
    /// - At t=300 (again): withdraw() returns 0 (already withdrawn)
    /// - At t=800: withdraw() returns 500 tokens (800 - 300 already withdrawn)
    /// - At t=1000: withdraw() returns 200 tokens, status → Completed
    pub fn withdraw(env: Env, stream_id: u64) -> Result<i128, ContractError> {
        require_not_globally_paused(&env)?;
        let mut stream = load_stream(&env, stream_id)?;

        // Enforce recipient-only authorization
        stream.recipient.require_auth();

        if stream.status == StreamStatus::Completed {
            return Err(ContractError::InvalidState);
        }

        if stream.status == StreamStatus::Paused && !is_terminal_state(&env, &stream) {
            return Err(ContractError::InvalidState);
        }

        let accrued = Self::calculate_accrued(env.clone(), stream_id)?;
        let mut withdrawable = accrued - stream.withdrawn_amount;

        // Cap by contract balance for safety (#39)
        let token_address = get_token(&env)?;
        let contract_balance =
            token::Client::new(&env, &token_address).balance(&env.current_contract_address());
        withdrawable = withdrawable.min(contract_balance);

        if withdrawable <= 0 {
            return Ok(0);
        }

        // Enforce dust threshold unless terminal state or final drain (#423)
        if withdrawable < stream.withdraw_dust_threshold
            && !is_terminal_state(&env, &stream)
            && stream.withdrawn_amount + withdrawable < stream.deposit_amount
        {
            return Ok(0);
        }

        // Enforce dust threshold unless terminal state or final drain (#423)
        if withdrawable < stream.withdraw_dust_threshold
            && !is_terminal_state(&env, &stream)
            && stream.withdrawn_amount + withdrawable < stream.deposit_amount
        {
            return Ok(0);
        }

        // CEI: update state before external token transfer to reduce reentrancy risk.
        // Assumption: the token contract does not reenter this contract.
        stream.withdrawn_amount += withdrawable;
        let completed_now = (stream.status == StreamStatus::Active
            || stream.status == StreamStatus::Paused)
            && stream.withdrawn_amount == stream.deposit_amount;
        if completed_now {
            stream.status = StreamStatus::Completed;
        }
        save_stream(&env, &stream);

        // Reduce liabilities as tokens leave the contract to the recipient.
        let liabilities = read_total_liabilities(&env)
            .checked_sub(withdrawable)
            .unwrap_or(0);
        write_total_liabilities(&env, liabilities);

        push_token(&env, &stream.recipient, withdrawable)?;

        env.events().publish(
            (symbol_short!("withdrew"), stream_id),
            Withdrawal {
                stream_id,
                recipient: stream.recipient.clone(),
                amount: withdrawable,
            },
        );

        if completed_now {
            env.events().publish(
                (symbol_short!("completed"), stream_id),
                StreamEvent::StreamCompleted(stream_id),
            );
        }

        Ok(withdrawable)
    }

    /// Withdraw accrued tokens from a payment stream to a specified destination address.
    ///
    /// Same accounting as [`withdraw`], but transfers tokens to `destination` instead of
    /// the stream's recipient. Use for wallet migration or custody workflows where the
    /// recipient wants tokens delivered to a different address (e.g. a cold wallet or
    /// a custody contract). The caller must still be the stream's recipient.
    ///
    /// # Parameters
    /// - `stream_id`: Unique identifier of the stream to withdraw from
    /// - `destination`: Address to receive the withdrawn tokens (must not be the contract itself)
    ///
    /// # Returns
    /// - `i128`: The amount of tokens transferred to `destination` (0 if nothing to withdraw)
    ///
    /// # Authorization
    /// - Requires authorization from the stream's `recipient` — the destination address is
    ///   not required to authorize. Only the stream's recipient may redirect funds.
    ///
    /// # Destination Constraints
    /// - `destination` must not equal `env.current_contract_address()`. Sending tokens back
    ///   to the contract would lock them permanently with no recovery path.
    /// - `destination` may equal the stream's `recipient` (self-redirect is allowed).
    /// - `destination` may be any other valid Stellar account or contract address.
    ///
    /// # Zero Withdrawable Behavior
    /// - If `accrued == withdrawn_amount` (nothing new to withdraw), returns 0 immediately.
    /// - No token transfer occurs, no state change, no event published.
    /// - This is idempotent: safe to call multiple times without side effects.
    /// - Occurs before cliff time or when all accrued funds have already been withdrawn.
    ///
    /// # State Changes
    /// - Updates `withdrawn_amount` by the amount transferred (only if withdrawable > 0).
    /// - Sets `status` to `Completed` if `withdrawn_amount` reaches `deposit_amount`.
    /// - Extends stream storage TTL to prevent expiration.
    ///
    /// # Events
    /// - Publishes `("wdraw_to", stream_id)` → `WithdrawalTo { stream_id, recipient, destination, amount }`
    ///   when `amount > 0`. The `recipient` field records who authorized the call; `destination`
    ///   records where tokens were sent — both are required for audit trails.
    /// - Publishes `("completed", stream_id)` → `StreamEvent::StreamCompleted(stream_id)`
    ///   immediately after the `WithdrawalTo` event if the stream is now fully drained.
    ///   Indexers must handle both events appearing in the same transaction.
    ///
    /// # Panics
    /// - `"destination must not be the contract"` — if `destination == current_contract_address()`
    /// - `"stream already completed"` — if stream status is `Completed`
    /// - `"cannot withdraw from paused stream"` — if stream status is `Paused`
    /// - If the stream does not exist (`StreamNotFound`)
    /// - If caller is not the stream's recipient (auth failure)
    ///
    /// # Usage Notes
    /// - Works on `Active` and `Cancelled` streams (same as `withdraw`).
    /// - For cancelled streams, only the accrued-but-not-yet-withdrawn amount is available;
    ///   the unstreamed refund was already returned to the sender at cancellation time.
    /// - CEI ordering: state is saved before the external token transfer to reduce reentrancy risk.
    pub fn withdraw_to(
        env: Env,
        stream_id: u64,
        destination: Address,
    ) -> Result<i128, ContractError> {
        require_not_globally_paused(&env)?;
        let mut stream = load_stream(&env, stream_id)?;

        // Enforce recipient-only authorization for source of funds
        stream.recipient.require_auth();

        if destination == env.current_contract_address() {
            return Err(ContractError::InvalidParams);
        }

        if stream.status == StreamStatus::Completed {
            return Err(ContractError::InvalidState);
        }

        if stream.status == StreamStatus::Paused && !is_terminal_state(&env, &stream) {
            return Err(ContractError::InvalidState);
        }

        let accrued = Self::calculate_accrued(env.clone(), stream_id)?;
        let mut withdrawable = accrued - stream.withdrawn_amount;

        // Cap by contract balance for safety (#39)
        let token_address = get_token(&env)?;
        let contract_balance =
            token::Client::new(&env, &token_address).balance(&env.current_contract_address());
        withdrawable = withdrawable.min(contract_balance);

        if withdrawable <= 0 {
            return Ok(0);
        }

        // Enforce dust threshold unless terminal state or final drain (#423)
        if withdrawable < stream.withdraw_dust_threshold
            && !is_terminal_state(&env, &stream)
            && stream.withdrawn_amount + withdrawable < stream.deposit_amount
        {
            return Ok(0);
        }

        stream.withdrawn_amount += withdrawable;
        let completed_now = (stream.status == StreamStatus::Active
            || stream.status == StreamStatus::Paused)
            && stream.withdrawn_amount == stream.deposit_amount;
        if completed_now {
            stream.status = StreamStatus::Completed;
        }
        save_stream(&env, &stream);

        // Reduce liabilities as tokens leave the contract.
        let liabilities = read_total_liabilities(&env)
            .checked_sub(withdrawable)
            .unwrap_or(0);
        write_total_liabilities(&env, liabilities);

        push_token(&env, &destination, withdrawable)?;

        env.events().publish(
            (symbol_short!("wdraw_to"), stream_id),
            WithdrawalTo {
                stream_id,
                recipient: stream.recipient.clone(),
                destination: destination.clone(),
                amount: withdrawable,
            },
        );

        if completed_now {
            env.events().publish(
                (symbol_short!("completed"), stream_id),
                StreamEvent::StreamCompleted(stream_id),
            );
        }

        Ok(withdrawable)
    }

    /// Rotate the receiving address for a stream.
    ///
    /// This allows the current recipient to transfer their entitlement to a new
    /// address (e.g. in case of a compromised wallet). Only the current recipient
    /// may authorize this rotation.
    ///
    /// # Parameters
    /// - `stream_id`: Unique identifier of the stream to update.
    /// - `new_recipient`: The new address that will receive the remaining streamed tokens.
    pub fn update_recipient(
        env: Env,
        stream_id: u64,
        new_recipient: Address,
    ) -> Result<(), ContractError> {
        require_not_globally_paused(&env)?;
        let mut stream = load_stream(&env, stream_id)?;

        // Only current recipient can authorize rotation
        stream.recipient.require_auth();

        if new_recipient == stream.recipient {
            return Err(ContractError::InvalidParams);
        }

        let old_recipient = stream.recipient.clone();

        // Update indices atomically
        remove_stream_from_recipient_index(&env, &old_recipient, stream_id);
        add_stream_to_recipient_index(&env, &new_recipient, stream_id);

        // Update state
        stream.recipient = new_recipient.clone();
        save_stream(&env, &stream);

        // Emit event
        env.events().publish(
            (symbol_short!("recp_upd"), stream_id),
            RecipientUpdated {
                stream_id,
                old_recipient,
                new_recipient,
            },
        );

        Ok(())
    }

    /// Withdraw accrued tokens from multiple streams in one call (recipient-only).
    ///
    /// The caller must be the recipient of every stream in `stream_ids`. Each stream
    /// is processed in order: same validation and accounting as `withdraw`. Events
    /// are emitted per stream. The operation is atomic: if any stream fails
    /// (e.g. not found, not recipient's, or paused), the entire call returns an error
    /// and no state changes or transfers occur.
    ///
    /// # Parameters
    /// - `recipient`: Address that must authorize and must be the recipient of all streams
    /// - `stream_ids`: Stream IDs to withdraw from (**must be unique**; duplicates return `DuplicateStreamId`)
    ///
    /// # Returns
    /// - `Vec<BatchWithdrawResult>`: Per-stream `(stream_id, amount)` for each entry.
    ///   `amount` is 0 for streams that are already `Completed` or have nothing to withdraw
    ///   (before cliff, or accrued == withdrawn). No token transfer or event is emitted for
    ///   those entries.
    ///
    /// # Empty Vector Semantics
    /// When `stream_ids` is empty:
    /// - Returns `Ok(Vec::new())` (empty result vector)
    /// - No streams are processed
    /// - No tokens are transferred
    /// - No events are emitted
    /// - Authorization is still required: `recipient.require_auth()` is called and must succeed
    /// - Contract state remains unchanged
    /// - No errors are raised (empty batch is valid)
    ///
    /// # Completed streams
    /// A `Completed` stream in the batch does **not** error. It contributes a zero-amount
    /// result and is skipped silently. This allows callers to pass a mixed list of active
    /// and already-completed streams without pre-filtering.
    ///
    /// # Zero Withdrawable Behavior
    /// - If an individual stream has `withdrawable == 0` (before cliff, or fully drained), it is skipped.
    /// - No token transfer, state modification, or event emission occurs for that specific stream.
    /// - The batch simply returns `amount: 0` for that stream in the `BatchWithdrawResult` array.
    ///
    /// # Authorization
    /// - Requires authorization from `recipient` once for the entire batch
    ///
    /// # Atomicity
    /// - All streams are processed in order. Any error (stream not found, wrong recipient,
    ///   paused, or duplicate IDs) reverts the whole transaction.
    /// - Completed streams are not an error: they produce amount `0` and no events.
    pub fn batch_withdraw(
        env: Env,
        recipient: Address,
        stream_ids: soroban_sdk::Vec<u64>,
    ) -> Result<soroban_sdk::Vec<BatchWithdrawResult>, ContractError> {
        require_not_globally_paused(&env)?;
        recipient.require_auth();

        let n = stream_ids.len();
        for i in 0..n {
            let a = stream_ids.get(i).unwrap();
            let mut j = i + 1;
            while j < n {
                if stream_ids.get(j).unwrap() == a {
                    return Err(ContractError::DuplicateStreamId);
                }
                j += 1;
            }
        }

        // Fetch initial contract balance and track remaining safety buffer (#39)
        let token_address = get_token(&env)?;
        let mut contract_balance =
            token::Client::new(&env, &token_address).balance(&env.current_contract_address());
        let mut results = soroban_sdk::Vec::new(&env);

        for stream_id in stream_ids.iter() {
            let mut stream = load_stream(&env, stream_id)?;

            if stream.recipient != recipient {
                return Err(ContractError::Unauthorized);
            }

            if stream.status == StreamStatus::Paused && !is_terminal_state(&env, &stream) {
                return Err(ContractError::InvalidState);
            }

            let mut withdrawable = if stream.status == StreamStatus::Completed {
                0
            } else {
                let accrued = Self::calculate_accrued(env.clone(), stream_id)?;
                (accrued - stream.withdrawn_amount).max(0)
            };

            // Cap by running contract balance for safety
            withdrawable = withdrawable.min(contract_balance);

            // Enforce dust threshold unless terminal state or final drain (#423)
            if withdrawable > 0
                && withdrawable < stream.withdraw_dust_threshold
                && !is_terminal_state(&env, &stream)
                && stream.withdrawn_amount + withdrawable < stream.deposit_amount
            {
                withdrawable = 0;
            }

            if withdrawable > 0 {
                // Decrement running balance before the transfer to ensure atomicity
                contract_balance -= withdrawable;

                stream.withdrawn_amount += withdrawable;
                let completed_now = (stream.status == StreamStatus::Active
                    || stream.status == StreamStatus::Paused)
                    && stream.withdrawn_amount == stream.deposit_amount;
                if completed_now {
                    stream.status = StreamStatus::Completed;
                }
                save_stream(&env, &stream);

                // Reduce liabilities as tokens leave the contract.
                let liabilities = read_total_liabilities(&env)
                    .checked_sub(withdrawable)
                    .unwrap_or(0);
                write_total_liabilities(&env, liabilities);

                push_token(&env, &stream.recipient, withdrawable)?;

                env.events().publish(
                    (symbol_short!("withdrew"), stream_id),
                    Withdrawal {
                        stream_id,
                        recipient: stream.recipient.clone(),
                        amount: withdrawable,
                    },
                );

                if completed_now {
                    env.events().publish(
                        (symbol_short!("completed"), stream_id),
                        StreamEvent::StreamCompleted(stream_id),
                    );
                }
            }

            results.push_back(BatchWithdrawResult {
                stream_id,
                amount: withdrawable,
            });
        }

        Ok(results)
    }

    /// Withdraw accrued tokens from multiple streams and route them to specified destinations.
    ///
    /// Similar to `batch_withdraw`, but allows the recipient to specify a distinct
    /// `destination` address for each stream withdrawal in the batch.
    ///
    /// The caller must be the recipient of every stream in `withdrawals`. The operation
    /// is atomic: if any stream fails (not found, unauthorized, paused, or invalid destination),
    /// the entire batch reverts.
    ///
    /// # Parameters
    /// - `recipient`: Address that must authorize and must be the recipient of all streams
    /// - `withdrawals`: List of `WithdrawToParam` (stream_id, destination). Stream IDs must be unique.
    ///
    /// # Returns
    /// - `Vec<BatchWithdrawResult>`: Per-stream `(stream_id, amount)` for each entry.
    pub fn batch_withdraw_to(
        env: Env,
        recipient: Address,
        withdrawals: soroban_sdk::Vec<WithdrawToParam>,
    ) -> Result<soroban_sdk::Vec<BatchWithdrawResult>, ContractError> {
        require_not_globally_paused(&env)?;
        recipient.require_auth();

        let n = withdrawals.len();
        for i in 0..n {
            let param_a = withdrawals.get(i).unwrap();

            if param_a.destination == env.current_contract_address() {
                return Err(ContractError::InvalidParams);
            }

            let mut j = i + 1;
            while j < n {
                let param_b = withdrawals.get(j).unwrap();
                assert!(
                    param_a.stream_id != param_b.stream_id,
                    "batch_withdraw_to stream_ids must be unique"
                );
                j += 1;
            }
        }

        // Fetch initial contract balance and track remaining safety buffer
        let token_address = get_token(&env)?;
        let mut contract_balance =
            token::Client::new(&env, &token_address).balance(&env.current_contract_address());

        let mut results = soroban_sdk::Vec::new(&env);

        for param in withdrawals.iter() {
            let mut stream = load_stream(&env, param.stream_id)?;

            if stream.recipient != recipient {
                return Err(ContractError::Unauthorized);
            }

            if stream.status == StreamStatus::Paused && !is_terminal_state(&env, &stream) {
                return Err(ContractError::InvalidState);
            }

            let mut withdrawable = if stream.status == StreamStatus::Completed {
                0
            } else {
                let accrued = Self::calculate_accrued(env.clone(), param.stream_id)?;
                (accrued - stream.withdrawn_amount).max(0)
            };

            // Cap by running contract balance for safety
            withdrawable = withdrawable.min(contract_balance);

            // Enforce dust threshold unless terminal state or final drain (#423)
            if withdrawable > 0
                && withdrawable < stream.withdraw_dust_threshold
                && !is_terminal_state(&env, &stream)
                && stream.withdrawn_amount + withdrawable < stream.deposit_amount
            {
                withdrawable = 0;
            }

            if withdrawable > 0 {
                contract_balance -= withdrawable;
                stream.withdrawn_amount += withdrawable;

                let completed_now = (stream.status == StreamStatus::Active
                    || stream.status == StreamStatus::Paused)
                    && stream.withdrawn_amount == stream.deposit_amount;
                if completed_now {
                    stream.status = StreamStatus::Completed;
                }
                save_stream(&env, &stream);

                push_token(&env, &param.destination, withdrawable)?;

                env.events().publish(
                    (symbol_short!("wdraw_to"), param.stream_id),
                    WithdrawalTo {
                        stream_id: param.stream_id,
                        recipient: stream.recipient.clone(),
                        destination: param.destination.clone(),
                        amount: withdrawable,
                    },
                );

                if completed_now {
                    env.events().publish(
                        (symbol_short!("completed"), param.stream_id),
                        StreamEvent::StreamCompleted(param.stream_id),
                    );
                }
            }

            results.push_back(BatchWithdrawResult {
                stream_id: param.stream_id,
                amount: withdrawable,
            });
        }

        Ok(results)
    }

    /// Calculate the total amount accrued to the recipient at the current time.
    ///
    /// # Behaviour by status
    ///
    /// | Status      | Return value                                         |
    /// |-------------|------------------------------------------------------|
    /// | `Active`    | `min((min(now,end)-start) × rate, deposit_amount)`   |
    /// | `Paused`    | Same time-based formula (accrual is not paused)      |
    /// | `Completed` | `deposit_amount` — all tokens were accrued/withdrawn |
    /// | `Cancelled` | Final accrued at cancellation timestamp (frozen value) |
    ///
    /// ## Rationale for `Cancelled`
    /// On cancellation, unstreamed tokens are refunded immediately to the sender.
    /// The recipient can claim only what was already accrued at cancellation time.
    /// Returning a frozen final accrued value keeps `calculate_accrued` consistent
    /// with contract balances and prevents post-cancel time growth.
    ///
    /// # Calculation
    /// - Before `cliff_time`: returns 0 (no accrual before cliff)
    /// - After `cliff_time`: `min((now - start_time) × rate_per_second, deposit_amount)`
    /// - After `end_time`: elapsed time is capped at `end_time` (no accrual beyond end)
    ///
    /// # Panics
    /// - If the stream does not exist (`stream_id` is invalid)
    ///
    /// # Usage Notes
    /// - This is a view function (read-only, no state changes)
    /// - No authorization required (public information)
    /// - Returns total accrued, not withdrawable amount
    /// - To get withdrawable amount: `calculate_accrued() - stream.withdrawn_amount`
    /// - Active/Paused streams accrue by current time; Completed/Cancelled are deterministic
    /// - Useful for UIs to show real-time accrual without transactions
    ///
    /// # Examples
    /// - Stream: 1000 tokens, 0-1000s, rate 1 token/sec, cliff at 500s
    /// - At t=300: returns 0 (before cliff)
    /// - At t=500: returns 500 (at cliff, accrual from start_time)
    /// - At t=800: returns 800
    /// - At t=1500: returns 1000 (elapsed time capped at end_time)
    /// ## Rationale for `Completed`
    /// When a stream reaches `Completed`, `withdrawn_amount == deposit_amount`.
    /// There is no further accrual possible. Returning `deposit_amount` is the
    /// deterministic, timestamp-independent answer for any UI or downstream caller.
    pub fn calculate_accrued(env: Env, stream_id: u64) -> Result<i128, ContractError> {
        let stream = load_stream(&env, stream_id)?;

        if stream.status == StreamStatus::Completed {
            return Ok(stream.deposit_amount);
        }

        let now = if stream.status == StreamStatus::Cancelled {
            stream.cancelled_at.ok_or(ContractError::InvalidState)?
        } else {
            env.ledger().timestamp()
        };

        Ok(accrual::calculate_accrued_amount_checkpointed(
            accrual::CheckpointState {
                checkpointed_amount: stream.checkpointed_amount,
                checkpointed_at: stream.checkpointed_at,
                cliff_time: stream.cliff_time,
                end_time: stream.end_time,
                deposit_amount: stream.deposit_amount,
            },
            stream.rate_per_second,
            now,
        ))
    }

    /// Calculate the currently withdrawable amount for a stream without performing a withdrawal.
    ///
    /// This is a read-only view function intended for UIs to display the "available to withdraw"
    /// balance. It mirrors the exact accrual and availability logic of `withdraw()`.
    ///
    /// # Parameters
    /// - `stream_id`: Unique identifier of the stream
    ///
    /// # Returns
    /// - `i128`: The amount currently available to withdraw.
    ///   - Returns `0` if the stream is `Paused` or `Completed` (withdraw is blocked).
    ///   - Returns `0` before the cliff time or when already fully withdrawn.
    ///   - For `Active` or `Cancelled` streams, this equals the amount `withdraw()` would return
    ///     at the current ledger time.
    ///
    /// # Errors
    /// - Returns `ContractError::StreamNotFound` if the stream does not exist.
    pub fn get_withdrawable(env: Env, stream_id: u64) -> Result<i128, ContractError> {
        let stream = load_stream(&env, stream_id)?;

        // If the stream is completed or paused, withdrawals are not allowed.
        if stream.status == StreamStatus::Completed || stream.status == StreamStatus::Paused {
            return Ok(0);
        }

        let accrued = Self::calculate_accrued(env.clone(), stream_id)?;
        let mut withdrawable = accrued - stream.withdrawn_amount;

        // Cap by contract balance for consistency with withdraw() (#39)
        let token_address = get_token(&env)?;
        let contract_balance =
            token::Client::new(&env, &token_address).balance(&env.current_contract_address());
        withdrawable = withdrawable.min(contract_balance);

        // Fallback max(0) just in case, though accrual is strictly monotonic
        Ok(if withdrawable > 0 { withdrawable } else { 0 })
    }

    /// Compute the claimable (withdrawable) amount at an arbitrary timestamp (read-only).
    ///
    /// Use this for simulation and planning: e.g. "how much could the recipient claim at
    /// time T?" without mutating state or using the current ledger time.
    ///
    /// # Parameters
    /// - `stream_id`: Unique identifier of the stream
    /// - `timestamp`: Ledger timestamp at which to evaluate claimable amount
    ///
    /// # Returns
    /// - `i128`: The amount that would be claimable (withdrawable) at the given timestamp.
    ///   Returns `0` for Completed streams, before cliff, or when already fully withdrawn.
    ///
    /// # Behaviour
    /// - **Active / Paused**: Accrual is computed at `timestamp` (clamped to stream schedule);
    ///   claimable = `max(0, accrued_at_timestamp - withdrawn_amount)`.
    /// - **Cancelled**: Accrual is frozen at cancellation; effective time is
    ///   `min(timestamp, cancelled_at)`, then same formula.
    /// - **Completed**: Returns `0` (nothing left to claim).
    ///
    /// # Errors
    /// - `ContractError::StreamNotFound` if the stream does not exist
    /// - `ContractError::InvalidState` if stream is Cancelled but `cancelled_at` is missing
    ///
    /// # Frontend usage
    /// - Call with a future timestamp to show "claimable at T" for planning.
    /// - Call with current ledger time to mirror `get_withdrawable` without state changes.
    pub fn get_claimable_at(
        env: Env,
        stream_id: u64,
        timestamp: u64,
    ) -> Result<i128, ContractError> {
        let stream = load_stream(&env, stream_id)?;

        if stream.status == StreamStatus::Completed {
            return Ok(0);
        }

        let effective_time = match stream.status {
            StreamStatus::Cancelled => {
                let at = stream.cancelled_at.ok_or(ContractError::InvalidState)?;
                timestamp.min(at)
            }
            StreamStatus::Active | StreamStatus::Paused => timestamp,
            StreamStatus::Completed => unreachable!("returned above"),
        };

        let accrued = accrual::calculate_accrued_amount_checkpointed(
            accrual::CheckpointState {
                checkpointed_amount: stream.checkpointed_amount,
                checkpointed_at: stream.checkpointed_at,
                cliff_time: stream.cliff_time,
                end_time: stream.end_time,
                deposit_amount: stream.deposit_amount,
            },
            stream.rate_per_second,
            effective_time,
        );

        let claimable = accrued - stream.withdrawn_amount;
        Ok(if claimable > 0 { claimable } else { 0 })
    }

    /// Retrieve the global contract configuration.
    ///
    /// Returns the contract's configuration containing the token address used for all
    /// streams and the admin address authorized for administrative operations.
    ///
    /// # Returns
    /// - `Config`: Structure containing:
    ///   - `token`: Address of the token contract used for all payment streams
    ///   - `admin`: Address authorized to perform admin operations (pause, cancel, resume)
    ///
    /// # Panics
    /// - If the contract has not been initialized (missing config)
    ///
    /// # Usage Notes
    /// - This is a view function (read-only, no state changes)
    /// - No authorization required (public information)
    /// - Config is set once during `init()` and can be updated via `set_admin()`
    /// - Useful for integrators to verify token and admin addresses
    pub fn get_config(env: Env) -> Result<Config, ContractError> {
        get_config(&env)
    }

    /// Returns `true` when the contract is in **global emergency pause**.
    ///
    /// In this mode, entrypoints guarded by `require_not_globally_paused` (stream
    /// creation, withdrawal, pause/resume/cancel, and schedule/rate updates) revert;
    /// views and admin maintenance entrypoints still run. `top_up_stream` is not
    /// currently gated by this flag.
    pub fn get_global_emergency_paused(env: Env) -> bool {
        is_global_emergency_paused(&env)
    }

    /// Update the admin address for the contract.
    ///
    /// Allows the current admin to rotate the admin key by setting a new admin address.
    /// This enables key rotation without redeploying the contract. Only the current admin
    /// may call this function.
    ///
    /// # Parameters
    /// - `new_admin`: The new admin address that will replace the current admin
    ///
    /// # Authorization
    /// - Requires authorization from the current admin address
    ///
    /// # Panics
    /// - If the contract has not been initialized (missing config)
    /// - If caller is not the current admin
    ///
    /// # State Changes
    /// - Updates the admin address in the Config stored in instance storage
    /// - Token address remains unchanged
    ///
    /// # Events
    /// - Publishes `AdminUpdated(old_admin, new_admin)` event on success
    ///
    /// # Usage Notes
    /// - This is a security-critical function for admin key rotation
    /// - The new admin immediately gains all administrative privileges
    /// - The old admin immediately loses all administrative privileges
    /// - No restrictions on the new admin address (can be any valid address)
    /// - Can be called multiple times to rotate keys as needed
    ///
    pub fn set_admin(env: Env, new_admin: Address) -> Result<(), ContractError> {
        let mut config = get_config(&env)?;
        let old_admin = config.admin.clone();

        // Only current admin can update admin
        old_admin.require_auth();

        // Update admin in config
        config.admin = new_admin.clone();
        env.storage().instance().set(&DataKey::Config, &config);

        // Bump TTL after instance write
        bump_instance_ttl(&env);

        // Emit event with old and new admin addresses
        env.events()
            .publish((soroban_sdk::Symbol::new(&env, "AdminUpdated"),), (old_admin, new_admin));

        Ok(())
    }

    /// Retrieve the complete state of a payment stream.
    ///
    /// Returns all stored information about a stream including participants, amounts,
    /// timing parameters, and current status. This is a read-only view function.
    ///
    /// # Parameters
    /// - `stream_id`: Unique identifier of the stream to query
    ///
    /// # Returns
    /// - `Stream`: Complete stream state containing:
    ///   - `stream_id`: Unique identifier
    ///   - `sender`: Address that created and funded the stream
    ///   - `recipient`: Address that receives the streamed tokens
    ///   - `deposit_amount`: Total tokens deposited (initial funding)
    ///   - `rate_per_second`: Streaming rate (tokens per second)
    ///   - `start_time`: When streaming begins (ledger timestamp)
    ///   - `cliff_time`: When tokens first become available (vesting cliff)
    ///   - `end_time`: When streaming completes (ledger timestamp)
    ///   - `withdrawn_amount`: Total tokens already withdrawn by recipient
    ///   - `status`: Current stream status (Active, Paused, Completed, Cancelled)
    ///
    /// # Panics
    /// - If the stream does not exist (`stream_id` is invalid)
    ///
    /// # Usage Notes
    /// - This is a view function (read-only, no state changes)
    /// - No authorization required (public information)
    /// - Useful for UIs to display stream details
    /// - Combine with `calculate_accrued()` to show real-time withdrawable amount
    /// - Status indicates current operational state:
    ///   - `Active`: Normal operation, recipient can withdraw
    ///   - `Paused`: Temporarily halted, no withdrawals allowed
    ///   - `Completed`: All tokens withdrawn, terminal state
    ///   - `Cancelled`: Terminated early, unstreamed tokens refunded, terminal state
    pub fn get_stream_state(env: Env, stream_id: u64) -> Result<Stream, ContractError> {
        load_stream(&env, stream_id)
    }

    /// Return the optional memo stored for a stream.
    ///
    /// Returns `None` when no memo was supplied at creation time or after the
    /// stream has been closed via `close_completed_stream`.
    ///
    /// # Errors
    /// - `ContractError::StreamNotFound` if the stream does not exist.
    pub fn get_stream_memo(
        env: Env,
        stream_id: u64,
    ) -> Result<Option<soroban_sdk::Bytes>, ContractError> {
        let stream = load_stream(&env, stream_id)?;
        Ok(stream.memo)
    }

    /// Return the total number of streams created so far.
    ///
    /// This value is backed by `NextStreamId`, which is incremented exactly once for
    /// each successful stream creation.
    pub fn get_stream_count(env: Env) -> u64 {
        read_stream_count(&env)
    }

    /// Update the `rate_per_second` of an existing stream.
    ///
    /// This is a **forward-only** rate change that preserves all existing invariants:
    ///
    /// - The stream must be in `Active` or `Paused` state (not terminal).
    /// - The caller must be the original stream sender.
    /// - The new rate must be **strictly greater** than the current rate.
    /// - The existing `deposit_amount` must still cover `new_rate × (end_time - start_time)`.
    ///
    /// Historical accrual is monotonic: at any given ledger time, the updated rate can
    /// only increase (never decrease) the accrued amount relative to the previous rate.
    /// This ensures the recipient's entitlement is never reduced by a rate update.
    ///
    /// # Parameters
    /// - `stream_id`: Unique identifier of the stream to update.
    /// - `new_rate_per_second`: New streaming rate in tokens per second (must be > current rate).
    ///
    /// # Returns
    /// - `Result<(), ContractError>`: `Ok(())` on success, or `StreamNotFound` on invalid `stream_id`.
    ///
    /// # Events
    /// - Emits a `rate_upd` event with a `RateUpdated` payload capturing old/new rate and effective time.
    pub fn update_rate_per_second(
        env: Env,
        stream_id: u64,
        new_rate_per_second: i128,
    ) -> Result<(), ContractError> {
        require_not_globally_paused(&env)?;
        let mut stream = load_stream(&env, stream_id)?;

        // Only the original sender can update the rate.
        Self::require_stream_sender(&stream.sender);

        // Only mutable (non-terminal) streams can be updated.
        if stream.status != StreamStatus::Active && stream.status != StreamStatus::Paused {
            return Err(ContractError::InvalidState);
        }

        if new_rate_per_second <= 0 {
            return Err(ContractError::InvalidParams);
        }

        let old_rate = stream.rate_per_second;
        // Forward-only semantics: disallow decreases (use decrease_rate_per_second for that).
        if new_rate_per_second <= old_rate {
            return Err(ContractError::InvalidParams);
        }

        // Validate that the existing deposit still covers the new total streamable amount.
        let duration = (stream.end_time - stream.start_time) as i128;
        let total_streamable = new_rate_per_second
            .checked_mul(duration)
            .ok_or(ContractError::ArithmeticOverflow)?;

        if stream.deposit_amount < total_streamable {
            return Err(ContractError::InsufficientDeposit);
        }

        // Checkpoint accrued-to-date so the rate increase applies forward-only.
        let now = env.ledger().timestamp();
        let accrued_now = accrual::calculate_accrued_amount_checkpointed(
            accrual::CheckpointState {
                checkpointed_amount: stream.checkpointed_amount,
                checkpointed_at: stream.checkpointed_at,
                cliff_time: stream.cliff_time,
                end_time: stream.end_time,
                deposit_amount: stream.deposit_amount,
            },
            old_rate,
            now,
        );
        stream.checkpointed_amount = accrued_now;
        stream.checkpointed_at = now;
        stream.rate_per_second = new_rate_per_second;
        save_stream(&env, &stream);

        env.events().publish(
            (symbol_short!("rate_upd"), stream_id),
            RateUpdated {
                stream_id,
                old_rate_per_second: old_rate,
                new_rate_per_second,
                effective_time: now,
            },
        );

        Ok(())
    }

    /// Safely decrease the streaming rate while preserving the recipient's accrued entitlement.
    ///
    /// This is the **safe decrease** counterpart to [`update_rate_per_second`] (which only
    /// permits increases). A naive rate decrease would retroactively reduce previously-accrued
    /// tokens, harming the recipient. This function prevents that by first taking a
    /// **checkpoint**: it locks in the mathematical accrual at the current moment under the
    /// old rate, then applies the new (lower) rate only for the remaining duration.
    ///
    /// ## Safety invariants proven
    ///
    /// 1. **Withdrawable never decreases**: immediately after the call, `calculate_accrued()`
    ///    returns exactly the same value as it did one instant before the call (the
    ///    `checkpointed_amount` is set to the pre-call accrual value and `checkpointed_at`
    ///    is set to `now`). Future accrual continues from this baseline.
    ///
    /// 2. **Total payable never exceeds deposit**: `new_deposit = checkpointed_amount +
    ///    new_rate × remaining_seconds`. The deposit is reduced to this amount and the
    ///    difference is refunded to the sender immediately.
    ///
    /// ## Parameters
    /// - `stream_id`: Unique identifier of the stream to update.
    /// - `new_rate_per_second`: New streaming rate in tokens per second.
    ///   Must satisfy `0 < new_rate < current rate_per_second`.
    ///
    /// ## Authorization
    /// - Requires authorization from the stream's original sender only.
    ///   Admin cannot call this; if an emergency rate cut is needed, use `cancel_stream_as_admin`.
    ///
    /// ## State Changes
    /// - `stream.checkpointed_amount` ← accrual at `now` under the old rate.
    /// - `stream.checkpointed_at` ← `now`.
    /// - `stream.rate_per_second` ← `new_rate_per_second`.
    /// - `stream.deposit_amount` ← `checkpointed_amount + new_rate × max(0, end_time − now)`.
    /// - Refunds `old_deposit − new_deposit` tokens to the sender.
    ///
    /// ## Returns
    /// - `Ok(())` on success.
    ///
    /// ## Errors
    /// - `StreamNotFound`      — `stream_id` does not exist.
    /// - `Unauthorized`        — caller is not the stream sender.
    /// - `StreamTerminalState` — stream is `Completed` or `Cancelled`.
    /// - `InvalidState`        — stream is past its `end_time` (already expired).
    /// - `InvalidParams`       — `new_rate_per_second <= 0` or `new_rate >= current_rate`.
    ///
    /// ## Events
    /// - Emits `("rate_dec", stream_id) → RateDecreased { ... }` on success.
    pub fn decrease_rate_per_second(
        env: Env,
        stream_id: u64,
        new_rate_per_second: i128,
    ) -> Result<(), ContractError> {
        require_not_globally_paused(&env)?;
        let mut stream = load_stream(&env, stream_id)?;

        // Sender-only: only the original creator may reduce the rate.
        Self::require_stream_sender(&stream.sender);

        // Terminal streams cannot be mutated.
        if stream.status == StreamStatus::Completed || stream.status == StreamStatus::Cancelled {
            return Err(ContractError::StreamTerminalState);
        }

        // Reject once the stream has expired; remaining duration would be zero.
        let now = env.ledger().timestamp();
        if now >= stream.end_time {
            return Err(ContractError::InvalidState);
        }

        // Validate the new rate: must be strictly positive and strictly less than the current rate.
        if new_rate_per_second <= 0 {
            return Err(ContractError::InvalidParams);
        }
        let old_rate = stream.rate_per_second;
        if new_rate_per_second >= old_rate {
            // Must use update_rate_per_second for increases.
            return Err(ContractError::InvalidParams);
        }

        // ── Checkpoint ────────────────────────────────────────────────────────────
        // Lock in accrual under the OLD rate at this exact instant.  Any value the
        // recipient could have withdrawn before this call remains reachable after.
        let accrued_now = accrual::calculate_accrued_amount_checkpointed(
            accrual::CheckpointState {
                checkpointed_amount: stream.checkpointed_amount,
                checkpointed_at: stream.checkpointed_at,
                cliff_time: stream.cliff_time,
                end_time: stream.end_time,
                deposit_amount: stream.deposit_amount,
            },
            old_rate,
            now,
        );

        // ── New deposit ceiling ────────────────────────────────────────────────────
        // Maximum tokens payable under the new rate:
        //   checkpoint + new_rate × remaining_seconds
        let remaining_seconds = (stream.end_time - now) as i128;
        let future_accrual = new_rate_per_second
            .checked_mul(remaining_seconds)
            .ok_or(ContractError::ArithmeticOverflow)?;
        let new_deposit = accrued_now
            .checked_add(future_accrual)
            .ok_or(ContractError::ArithmeticOverflow)?;

        // new_deposit must fit within the old deposit (guaranteed by lower rate * same duration).
        let old_deposit = stream.deposit_amount;
        let refund_amount = old_deposit
            .checked_sub(new_deposit)
            .ok_or(ContractError::ArithmeticOverflow)?;

        // Sanity: refund must be non-negative (lower rate → smaller max payable).
        if refund_amount < 0 {
            return Err(ContractError::InvalidState);
        }

        // ── CEI: persist state before token transfer ───────────────────────────────
        stream.checkpointed_amount = accrued_now;
        stream.checkpointed_at = now;
        stream.rate_per_second = new_rate_per_second;
        stream.deposit_amount = new_deposit;
        save_stream(&env, &stream);

        // Refund the now-unreachable portion of the deposit to the sender.
        if refund_amount > 0 {
            push_token(&env, &stream.sender, refund_amount)?;
        }

        env.events().publish(
            (symbol_short!("rate_dec"), stream_id),
            RateDecreased {
                stream_id,
                old_rate_per_second: old_rate,
                new_rate_per_second,
                effective_time: now,
                checkpointed_amount: accrued_now,
                refund_amount,
            },
        );

        Ok(())
    }

    /// Shorten a stream's `end_time` and refund unstreamed tokens to the sender.
    ///
    /// This operation safely reduces the remaining duration of an **Active** or **Paused**
    /// stream while:
    ///
    /// - Preserving all already-accrued entitlement for the recipient.
    /// - Refunding only the portion of the deposit that can never accrue under the new end time.
    /// - Maintaining the invariant `deposit_amount >= accrued(now)` at the moment of update.
    ///
    /// # Parameters
    /// - `stream_id`: Unique identifier of the stream to update.
    /// - `new_end_time`: New stream end timestamp (must be:
    ///   - `> current_ledger_timestamp`
    ///   - `> start_time`
    ///   - `>= cliff_time`
    ///   - `< current end_time`).
    ///
    /// # Behaviour
    /// - Computes the new maximum streamable amount as
    ///   `rate_per_second × (new_end_time - start_time)`.
    /// - Sets `deposit_amount` to this new maximum streamable amount.
    /// - Refunds `old_deposit - new_deposit` to the sender.
    /// - Leaves accrued amount at the current ledger time unchanged.
    ///
    /// # Returns
    /// - `Result<(), ContractError>`: `Ok(())` on success, or `StreamNotFound` on invalid `stream_id`.
    ///
    /// # Events
    /// - Emits a `sched_shrt` event with a `StreamEndShortened` payload describing the change.
    pub fn shorten_stream_end_time(
        env: Env,
        stream_id: u64,
        new_end_time: u64,
    ) -> Result<(), ContractError> {
        require_not_globally_paused(&env)?;
        let mut stream = load_stream(&env, stream_id)?;

        // Only the original sender can modify the schedule.
        Self::require_stream_sender(&stream.sender);

        // Only non-terminal streams may be shortened.
        Self::require_cancellable_status(stream.status)?;

        let now = env.ledger().timestamp();

        // New end time must move strictly earlier and remain strictly in the future.
        if new_end_time <= now
            || new_end_time <= stream.start_time
            || new_end_time < stream.cliff_time
            || new_end_time >= stream.end_time
        {
            return Err(ContractError::InvalidParams);
        }

        // Compute new maximum streamable amount under the shortened schedule.
        let new_duration = (new_end_time - stream.start_time) as i128;
        let new_max_streamable = stream
            .rate_per_second
            .checked_mul(new_duration)
            .ok_or(ContractError::ArithmeticOverflow)?;

        // Deposit must still be sufficient to cover the shortened schedule (by construction
        // this should hold given the original validation, but we keep an explicit assert).
        if new_max_streamable > stream.deposit_amount {
            return Err(ContractError::InvalidParams);
        }

        let old_end_time = stream.end_time;
        let old_deposit = stream.deposit_amount;
        let refund_amount = old_deposit
            .checked_sub(new_max_streamable)
            .ok_or(ContractError::ArithmeticOverflow)?;

        stream.end_time = new_end_time;
        stream.deposit_amount = new_max_streamable;
        save_stream(&env, &stream);

        if refund_amount > 0 {
            // Reduce liabilities by the refunded portion (no longer owed to recipient).
            let liabilities = read_total_liabilities(&env)
                .checked_sub(refund_amount)
                .unwrap_or(0);
            write_total_liabilities(&env, liabilities);
            push_token(&env, &stream.sender, refund_amount)?;
        }

        env.events().publish(
            (symbol_short!("end_shrt"), stream_id),
            StreamEndShortened {
                stream_id,
                old_end_time,
                new_end_time,
                refund_amount,
            },
        );

        Ok(())
    }

    /// Extend a stream's `end_time` without changing its deposit or rate.
    ///
    /// This operation lengthens the schedule of an **Active** or **Paused** stream while:
    ///
    /// - Keeping the rate and deposit fixed.
    /// - Ensuring the existing `deposit_amount` still safely covers the extended duration.
    /// - Preserving accrued amount at the current ledger time.
    ///
    /// # Parameters
    /// - `stream_id`: Unique identifier of the stream to update.
    /// - `new_end_time`: New stream end timestamp (must be:
    ///   - `> current end_time`
    ///   - `> start_time`
    ///   - `>= cliff_time`
    ///   - `>= current_ledger_timestamp`).
    ///
    /// # Behaviour
    /// - Validates `deposit_amount >= rate_per_second × (new_end_time - start_time)`.
    /// - Updates `end_time` in-place; all other fields remain unchanged.
    /// - Accrual at the current ledger time is unchanged; future accrual continues linearly.
    ///
    /// # Returns
    /// - `Result<(), ContractError>`: `Ok(())` on success, or `StreamNotFound` on invalid `stream_id`.
    ///
    /// # Events
    /// - Emits an `end_ext` event with a `StreamEndExtended` payload describing the change.
    pub fn extend_stream_end_time(
        env: Env,
        stream_id: u64,
        new_end_time: u64,
    ) -> Result<(), ContractError> {
        require_not_globally_paused(&env)?;
        let mut stream = load_stream(&env, stream_id)?;

        // Only the original sender can modify the schedule.
        Self::require_stream_sender(&stream.sender);

        // Only non-terminal streams may be extended.
        Self::require_cancellable_status(stream.status)?;

        let now = env.ledger().timestamp();

        // Must move end_time forward in time.
        if new_end_time <= stream.end_time
            || new_end_time <= stream.start_time
            || new_end_time < stream.cliff_time
            || new_end_time < now
        {
            return Err(ContractError::InvalidParams);
        }

        // Ensure existing deposit still covers the extended schedule at the current rate.
        let new_duration = (new_end_time - stream.start_time) as i128;
        let new_total_streamable = stream
            .rate_per_second
            .checked_mul(new_duration)
            .ok_or(ContractError::ArithmeticOverflow)?;

        if new_total_streamable > stream.deposit_amount {
            return Err(ContractError::InsufficientDeposit);
        }

        let old_end_time = stream.end_time;
        stream.end_time = new_end_time;
        save_stream(&env, &stream);

        env.events().publish(
            (symbol_short!("end_ext"), stream_id),
            StreamEndExtended {
                stream_id,
                old_end_time,
                new_end_time,
            },
        );

        Ok(())
    }

    /// Increase the deposit amount of an existing stream.
    ///
    /// This operation **tops up** the locked funding backing a stream without changing
    /// its schedule (`start_time`, `cliff_time`, `end_time`) or rate. It is intended
    /// for treasury operations that want to increase the total allocation for an
    /// existing agreement.
    ///
    /// # Parameters
    /// - `stream_id`: Unique identifier of the stream to top up.
    /// - `funder`: Address providing the additional tokens. Must be the original
    ///   stream sender or the contract admin.
    /// - `amount`: Additional amount of tokens to lock into the stream (must be > 0).
    ///
    /// # Authorization
    /// - Requires authorization from `funder`.
    /// - No sender/admin relationship is enforced on-chain: any address may top up
    ///   if it signs the call and can transfer the requested token amount.
    ///
    /// # Behaviour
    /// - Increases `deposit_amount` by `amount` (with overflow protection).
    /// - Persists the increased deposit before calling the token contract to pull
    ///   `amount` from `funder`.
    /// - Does **not** modify `rate_per_second` or any timing fields.
    /// - Leaves `status`, `withdrawn_amount`, and all schedule fields unchanged.
    ///
    /// # Restrictions
    /// - Only streams in `Active` or `Paused` status can be topped up.
    /// - `amount` must be strictly positive.
    /// - `current_ledger_time` must be strictly less than `end_time`.
    ///
    /// # CEI Pattern
    /// State is persisted **before** the external token pull to prevent reentrancy.
    ///
    /// # Returns
    /// - `Ok(())` on success.
    /// - `Err(StreamNotFound)` if `stream_id` does not exist.
    /// - `Err(InvalidParams)` if `amount <= 0`.
    /// - `Err(InvalidState)` if the stream is not `Active` or `Paused`.
    /// - `Err(ArithmeticOverflow)` if `deposit_amount + amount` exceeds `i128::MAX`.
    ///
    /// # Failure Semantics
    /// - If auth fails or the token transfer fails, the transaction reverts atomically:
    ///   no deposit increase persists and no `top_up` event is emitted.
    ///
    /// # Events
    /// - Emits a `top_up` event with `StreamToppedUp` payload on success.
    pub fn top_up_stream(
        env: Env,
        stream_id: u64,
        funder: Address,
        amount: i128,
    ) -> Result<(), ContractError> {
        require_not_globally_paused(&env)?;
        // --- Checks ---
        if amount <= 0 {
            return Err(ContractError::InvalidParams);
        }

        let stream = load_stream(&env, stream_id)?;

        if stream.status != StreamStatus::Active && stream.status != StreamStatus::Paused {
            return Err(ContractError::InvalidState);
        }

        // Reject top-ups on expired streams to prevent zombie fund lock-up.
        // Even if submitted in the same block as expiry, no seconds remain to
        // stream the new funds, so the deposit would be permanently unclaimable.
        let now = env.ledger().timestamp();
        if now >= stream.end_time {
            return Err(ContractError::InvalidState);
        }

        // Allow any authorized address to top up (third-party funding support).
        funder.require_auth();

        // --- Effects ---
        // Increase deposit_amount with overflow protection.
        let new_deposit = stream
            .deposit_amount
            .checked_add(amount)
            .ok_or(ContractError::ArithmeticOverflow)?; // overflow

        let new_end_time = stream.end_time;

        // Persist updated state BEFORE the external token pull (CEI).
        let mut stream = stream;
        stream.deposit_amount = new_deposit;
        save_stream(&env, &stream);

        // --- Interactions ---
        pull_token(&env, &funder, amount)?;

        // Increase liabilities to match the additional deposit.
        let liabilities = read_total_liabilities(&env)
            .checked_add(amount)
            .unwrap_or(i128::MAX);
        write_total_liabilities(&env, liabilities);

        env.events().publish(
            (symbol_short!("top_up"), stream_id),
            StreamToppedUp {
                stream_id,
                top_up_amount: amount,
                new_deposit_amount: new_deposit,
                new_end_time,
            },
        );
        Ok(())
    }

    /// Close (archive) a completed stream to reduce long-term storage.
    ///
    /// Permanently removes the stream's persistent storage entry. Only streams in
    /// `Completed` status can be closed; all payouts must already have been made.
    /// After close, the stream is no longer queryable (`get_stream_state` returns
    /// `StreamNotFound`).
    ///
    /// # Parameters
    /// - `stream_id`: Unique identifier of the stream to close
    ///
    /// # Returns
    /// - `Result<(), ContractError>`: `Ok(())` on success
    ///
    /// # Preconditions
    /// - Stream must exist and have status `Completed`
    ///
    /// # Panics
    /// - If the stream does not exist
    /// - If the stream is not `Completed` (Active, Paused, or Cancelled)
    ///
    /// # Events
    /// - Publishes `closed(stream_id)` with `StreamEvent::StreamClosed(stream_id)` before removal.
    ///
    /// # Operational guidance
    /// - Callable by anyone; no authorization required (permissionless cleanup).
    /// - Not blocked by global emergency pause (storage hygiene only).
    /// - Indexers and UIs should treat closed stream IDs as non-existent.
    /// - Do not close streams that might still need historical data for accounting.
    pub fn close_completed_stream(env: Env, stream_id: u64) -> Result<(), ContractError> {
        let stream = load_stream(&env, stream_id)?;

        // Only explicitly terminal streams (Completed or Cancelled) can be closed.
        if stream.status != StreamStatus::Completed && stream.status != StreamStatus::Cancelled {
            return Err(ContractError::InvalidState);
        }

        // For Cancelled streams, prove no claimable balance remains before removing.
        // Accrual is frozen at cancelled_at; the recipient may still withdraw the frozen amount.
        // Closing before full settlement would destroy recipient funds.
        if stream.status == StreamStatus::Cancelled {
            let cancelled_at = stream.cancelled_at.ok_or(ContractError::InvalidState)?;
            let accrued = accrual::calculate_accrued_amount_checkpointed(
                accrual::CheckpointState {
                    checkpointed_amount: stream.checkpointed_amount,
                    checkpointed_at: stream.checkpointed_at,
                    cliff_time: stream.cliff_time,
                    end_time: stream.end_time,
                    deposit_amount: stream.deposit_amount,
                },
                stream.rate_per_second,
                cancelled_at,
            );
            let claimable = accrued.saturating_sub(stream.withdrawn_amount).max(0);
            if claimable > 0 {
                return Err(ContractError::InvalidState);
            }
        }

        env.events().publish(
            (symbol_short!("closed"), stream_id),
            StreamEvent::StreamClosed(stream_id),
        );

        // Remove stream from recipient's index before deleting the stream
        remove_stream_from_recipient_index(&env, &stream.recipient, stream_id);
        remove_stream(&env, stream_id);

        Ok(())
    }

    /// Register a reusable relative schedule (start/cliff/duration offsets only).
    ///
    /// Caps: [`MAX_TEMPLATES_PER_OWNER`] per registering address and [`MAX_GLOBAL_TEMPLATES`]
    /// across all owners. Only `owner` may delete via [`Self::delete_stream_template`].
    pub fn register_stream_template(
        env: Env,
        owner: Address,
        start_delay: u64,
        cliff_delay: u64,
        duration: u64,
    ) -> Result<u64, ContractError> {
        owner.require_auth();
        validate_template_delays(&env, start_delay, cliff_delay, duration)?;
        let ids = load_owner_template_ids(&env, &owner);
        if ids.len() >= MAX_TEMPLATES_PER_OWNER {
            return Err(ContractError::TemplateLimitExceeded);
        }
        let active = read_active_template_count(&env);
        if active >= MAX_GLOBAL_TEMPLATES {
            return Err(ContractError::TemplateLimitExceeded);
        }
        let template_id = read_next_template_id(&env);
        let tpl = StreamScheduleTemplate {
            template_id,
            owner: owner.clone(),
            start_delay,
            cliff_delay,
            duration,
        };
        save_stream_template(&env, &tpl);
        let mut new_ids = ids;
        new_ids.push_back(template_id);
        save_owner_template_ids(&env, &owner, &new_ids);
        set_next_template_id(&env, template_id + 1);
        set_active_template_count(&env, active + 1);
        env.events()
            .publish((symbol_short!("tmpl_def"), template_id), tpl.clone());
        Ok(template_id)
    }

    /// Delete a schedule template. Only the registering `owner` may call.
    pub fn delete_stream_template(
        env: Env,
        owner: Address,
        template_id: u64,
    ) -> Result<(), ContractError> {
        owner.require_auth();
        let tpl = load_stream_template(&env, template_id)?;
        if tpl.owner != owner {
            return Err(ContractError::TemplateUnauthorized);
        }
        remove_stream_template_storage(&env, template_id);
        remove_template_id_for_owner(&env, &owner, template_id)?;
        let active = read_active_template_count(&env);
        set_active_template_count(&env, active.saturating_sub(1));
        Ok(())
    }

    /// Create a stream using a registered template's relative timing plus caller-funded amounts.
    pub fn create_stream_from_template(
        env: Env,
        sender: Address,
        template_id: u64,
        recipient: Address,
        deposit_amount: i128,
        rate_per_second: i128,
        withdraw_dust_threshold: i128,
        memo: Option<soroban_sdk::Bytes>,
    ) -> Result<u64, ContractError> {
        let tpl = load_stream_template(&env, template_id)?;
        Self::create_stream_relative(
            env,
            sender,
            CreateStreamRelativeParams {
                recipient,
                deposit_amount,
                rate_per_second,
                start_delay: tpl.start_delay,
                cliff_delay: tpl.cliff_delay,
                duration: tpl.duration,
                withdraw_dust_threshold: Some(withdraw_dust_threshold),
                memo,
            },
        )
    }

    /// Read a schedule template by id (permissionless view).
    pub fn get_stream_template(
        env: Env,
        template_id: u64,
    ) -> Result<StreamScheduleTemplate, ContractError> {
        load_stream_template(&env, template_id)
    }

    /// Return the compile-time contract version number.
    ///
    /// This is a permissionless, read-only entry-point that returns the value of
    /// [`CONTRACT_VERSION`]. No storage access is performed; the value is embedded
    /// in the WASM binary at compile time.
    ///
    /// # Returns
    /// - `u32`: The current contract version (currently `1`)
    ///
    /// # Authorization
    /// - None required. Any caller (wallet, indexer, script) may call this.
    ///
    /// # Usage
    /// Deployment scripts and integrators should call `version()` immediately after
    /// obtaining a contract address to confirm the expected protocol revision is
    /// running before sending any state-mutating transactions.
    ///
    /// ```text
    /// assert version() == EXPECTED_VERSION, "wrong contract version"
    /// ```
    ///
    /// # Availability
    /// `version()` works even on an uninitialised contract (before `init` is called).
    /// This allows pre-flight version checks during deployment pipelines.
    ///
    /// # Gas
    /// Minimal — no storage reads, no token interactions.
    pub fn version(_env: Env) -> u32 {
        CONTRACT_VERSION
    }

    /// Retrieve all stream IDs for a given recipient (sorted by stream_id).
    ///
    /// Returns a vector of stream IDs where the recipient is the stream's recipient address.
    /// The list is maintained in sorted ascending order by stream_id for deterministic
    /// pagination and UI display. This enables efficient recipient portal workflows where
    /// users can see all their incoming streams.
    ///
    /// # Parameters
    /// - `recipient`: Address to query streams for
    ///
    /// # Returns
    /// - `Vec<u64>`: Vector of stream IDs (sorted ascending by stream_id)
    ///   - Empty vector if the recipient has no streams
    ///   - Includes streams in all statuses (Active, Paused, Completed, Cancelled)
    ///   - Does not include closed streams (removed via `close_completed_stream`)
    ///
    /// # Behavior
    /// - This is a view function (read-only, no state changes)
    /// - No authorization required (public information)
    /// - Extends TTL on the recipient's index to prevent expiration
    /// - Useful for recipient portals to enumerate all streams
    /// - Can be used for pagination by combining with `get_stream_state`
    ///
    /// # Consistency Guarantees
    /// - **Sorted order**: Always returns streams in ascending order by stream_id
    /// - **Completeness**: Includes all active streams for the recipient
    /// - **Lifecycle consistency**: Streams are added on creation, removed on close
    /// - **Recipient updates**: If recipient changes (not currently supported), index remains consistent
    ///
    /// # Usage Notes
    /// - Combine with `get_stream_state` to fetch full stream details
    /// - Use with `calculate_accrued` to show real-time balances
    /// - For large recipient portfolios, consider pagination strategies
    /// - Closed streams are not included (use `get_stream_state` to verify existence)
    ///
    /// # Examples
    /// - Get all streams for a recipient: `get_recipient_streams(env, recipient_address)`
    /// - Paginate: fetch first N IDs, then call `get_stream_state` for each
    /// - Filter by status: fetch all IDs, then check status of each via `get_stream_state`
    pub fn get_recipient_streams(env: Env, recipient: Address) -> soroban_sdk::Vec<u64> {
        load_recipient_streams(&env, &recipient)
    }

    /// Count the total number of streams for a recipient.
    /// # Parameters
    /// - `recipient`: Address to query stream count for
    ///
    /// # Returns
    /// - `u64`: Number of streams for the recipient (0 if none)
    ///
    /// # Behavior
    /// - This is a view function (read-only, no state changes)
    /// - No authorization required (public information)
    /// - Extends TTL on the recipient's index to prevent expiration
    /// - More gas-efficient than `get_recipient_streams` when only count is needed
    ///
    /// # Usage Notes
    /// - Use for UI indicators (e.g., "You have 5 active streams")
    /// - Combine with `get_recipient_streams` for pagination
    /// - Closed streams are not included in the count
    pub fn get_recipient_stream_count(env: Env, recipient: Address) -> u64 {
        load_recipient_streams(&env, &recipient).len() as u64
    }

    /// Export streams by ID range with bounded page size (operator migration support).
    ///
    /// Returns a paginated list of streams within the specified ID range `[start_id, end_id]`.
    /// This enables efficient, bounded data export for off-chain migration between contract
    /// instances without unbounded loops or memory exhaustion.
    ///
    /// # Parameters
    /// - `start_id`: First stream ID to include in the range (inclusive)
    /// - `end_id`: Last stream ID to include in the range (inclusive). Use `u64::MAX` for open-ended.
    /// - `limit`: Maximum number of streams to return (capped at [`MAX_PAGE_SIZE`])
    ///
    /// # Returns
    /// - `Vec<Stream>`: Vector of stream structs in ascending order by stream_id
    ///   - Empty if no streams exist in the range
    ///   - Partial results if some stream IDs in range don't exist (deleted/closed)
    ///   - Length never exceeds `min(limit, MAX_PAGE_SIZE)`
    ///
    /// # Pagination Strategy
    /// For complete export across all streams:
    /// 1. Call `get_stream_count()` to get total stream count
    /// 2. Iterate in chunks: `get_streams_by_id_range(1, 100, 100)`, `get_streams_by_id_range(101, 200, 100)`, etc.
    /// 3. Handle missing IDs gracefully (some may be closed/archived)
    ///
    /// # DoS Protection
    /// - `limit` is strictly capped at [`MAX_PAGE_SIZE`] (100)
    /// - Range size is bounded by `limit`, not `end_id - start_id`
    /// - Each stream lookup is O(1), total gas is O(limit)
    ///
    /// # Errors
    /// - Returns empty vector if `start_id > end_id`
    /// - Non-existent stream IDs are silently skipped
    ///
    /// # Example
    /// ```ignore
    /// // Export first 50 streams (IDs 1-50)
    /// let streams = get_streams_by_id_range(&env, 1, 50, 50);
    ///
    /// // Export next page using open-ended range
    /// let streams = get_streams_by_id_range(&env, 51, u64::MAX, 100);
    /// ```
    pub fn get_streams_by_id_range(
        env: Env,
        start_id: u64,
        end_id: u64,
        limit: u64,
    ) -> soroban_sdk::Vec<Stream> {
        // Enforce DoS protection limit
        let page_size = limit.min(MAX_PAGE_SIZE);
        let mut result = soroban_sdk::Vec::new(&env);

        // Handle invalid range
        if start_id > end_id || page_size == 0 {
            return result;
        }

        let total_count = read_stream_count(&env);
        let effective_end = end_id.min(total_count);

        let mut current_id = start_id;
        while current_id <= effective_end && result.len() < page_size as u32 {
            // Try to load stream, skip if not found (closed/archived)
            if let Ok(stream) = load_stream(&env, current_id) {
                result.push_back(stream);
            }
            current_id += 1;
        }

        result
    }

    /// Paginated export of recipient streams with cursor-based pagination.
    ///
    /// Returns a bounded page of stream IDs for a recipient starting from a cursor position.
    /// Designed for efficient, resumable export of large recipient portfolios without
    /// unbounded memory usage.
    ///
    /// # Parameters
    /// - `recipient`: Address to query streams for
    /// - `cursor`: Starting position in the recipient's stream list (0-based index).
    ///   Use 0 for first page, then `cursor + previous_result.len()` for next page.
    /// - `limit`: Maximum number of streams to return (capped at [`MAX_PAGE_SIZE`])
    ///
    /// # Returns
    /// - `Vec<u64>`: Vector of stream IDs in ascending order
    ///   - Empty vector if `cursor >= recipient_stream_count`
    ///   - Length never exceeds `min(limit, MAX_PAGE_SIZE)`
    ///
    /// # Cursor Semantics
    /// - Cursor is a 0-based index into the sorted recipient stream list
    /// - After each call, next cursor = `cursor + result.len()`
    /// - When result.len() < limit, you've reached the end
    /// - Cursor survives stream list mutations (insertions/removals shift indices naturally)
    ///
    /// # Pagination Strategy
    /// ```ignore
    /// let mut cursor = 0;
    /// let page_size = 50;
    /// loop {
    ///     let page = get_recipient_streams_paginated(&env, &recipient, cursor, page_size);
    ///     if page.is_empty() { break; }
    ///     // Process page...
    ///     cursor += page.len();
    /// }
    /// ```
    ///
    /// # DoS Protection
    /// - `limit` is strictly capped at [`MAX_PAGE_SIZE`] (100)
    /// - Cursor-based pagination prevents unbounded list traversal
    /// - Gas cost is O(limit) regardless of recipient's total stream count
    ///
    /// # Consistency Guarantees
    /// - Stream list is sorted by stream_id (ascending)
    /// - Pagination is stable: repeated calls with same cursor return same results
    ///   unless the underlying list is modified
    /// - New streams added during pagination may appear or not depending on insertion position
    pub fn get_recipient_streams_paginated(
        env: Env,
        recipient: Address,
        cursor: u64,
        limit: u64,
    ) -> soroban_sdk::Vec<u64> {
        // Enforce DoS protection limit
        let page_size = limit.min(MAX_PAGE_SIZE) as u32;
        let all_streams = load_recipient_streams(&env, &recipient);
        let total = all_streams.len() as u64;

        // Return empty if cursor is beyond end
        if cursor >= total || page_size == 0 {
            return soroban_sdk::Vec::new(&env);
        }

        let start_idx = cursor as u32;
        let available = total as u32 - start_idx;
        let take_count = page_size.min(available);

        let mut result = soroban_sdk::Vec::new(&env);
        for i in 0..take_count {
            if let Some(stream_id) = all_streams.get(start_idx + i) {
                result.push_back(stream_id);
            }
        }

        result
    }

    /// Internal helper to require authorization from the stream sender.
    ///
    /// Admin override paths are handled by dedicated `*_as_admin` entrypoints.
    fn require_stream_sender(sender: &Address) {
        sender.require_auth();
    }

    fn require_cancellable_status(status: StreamStatus) -> Result<(), ContractError> {
        if status != StreamStatus::Active && status != StreamStatus::Paused {
            return Err(ContractError::InvalidState);
        }
        Ok(())
    }

    /// Shared cancellation implementation for sender/admin entrypoints.
    ///
    /// Guarantees identical externally visible behavior across both auth paths:
    /// - same state transition (`status = Cancelled`, `cancelled_at = now`)
    /// - same refund rule (`refund = deposit_amount - accrued_at_now`)
    /// - same event shape (`StreamCancelled(stream_id)`)
    fn cancel_stream_internal(env: &Env, stream: &mut Stream) -> Result<(), ContractError> {
        Self::require_cancellable_status(stream.status)?;

        let now = env.ledger().timestamp();
        // Use checkpoint-aware accrual so rate-decreased streams are cancelled correctly.
        let accrued_at_cancel = accrual::calculate_accrued_amount_checkpointed(
            accrual::CheckpointState {
                checkpointed_amount: stream.checkpointed_amount,
                checkpointed_at: stream.checkpointed_at,
                cliff_time: stream.cliff_time,
                end_time: stream.end_time,
                deposit_amount: stream.deposit_amount,
            },
            stream.rate_per_second,
            now,
        );

        let refund_amount = stream
            .deposit_amount
            .checked_sub(accrued_at_cancel)
            .ok_or(ContractError::InvalidState)?;

        // CEI: persist terminal state before external token transfer.
        stream.status = StreamStatus::Cancelled;
        stream.cancelled_at = Some(now);
        save_stream(env, stream);

        // Reduce liabilities by the refunded (unstreamed) portion.
        // The accrued portion remains a liability until the recipient withdraws.
        if refund_amount > 0 {
            let liabilities = read_total_liabilities(env)
                .checked_sub(refund_amount)
                .unwrap_or(0);
            write_total_liabilities(env, liabilities);
            push_token(env, &stream.sender, refund_amount)?;
        }

        env.events().publish(
            (symbol_short!("cancelled"), stream.stream_id),
            StreamEvent::StreamCancelled(stream.stream_id),
        );

        Ok(())
    }

    pub fn update_rate(
        env: Env,
        stream_id: u64,
        new_rate_per_second: i128,
        caller: Address,
    ) -> Result<(), ContractError> {
        // Authorization
        caller.require_auth();

        // Load stream
        let mut stream = load_stream(&env, stream_id)?;

        // Reject terminal states
        if stream.status == StreamStatus::Completed || stream.status == StreamStatus::Cancelled {
            return Err(ContractError::StreamTerminalState);
        }

        // Only sender or admin can update rate
        let admin = get_admin(&env)?;
        if caller != stream.sender && caller != admin {
            return Err(ContractError::Unauthorized);
        }

        // Validate new rate
        if new_rate_per_second <= 0 {
            return Err(ContractError::InvalidParams);
        }

        let old_rate = stream.rate_per_second;

        // 🔑 IMPORTANT: Do NOT touch withdrawn_amount
        // This preserves correctness after partial withdrawals
        stream.rate_per_second = new_rate_per_second;

        // Save updated stream
        save_stream(&env, &stream);

        // Emit event
        env.events().publish(
            (symbol_short!("rate_upd"), stream_id),
            RateUpdated {
                stream_id,
                old_rate_per_second: old_rate,
                new_rate_per_second,
                effective_time: env.ledger().timestamp(),
            },
        );

        Ok(())
    }

    /// Cancel a payment stream as the contract admin.
    ///
    /// Administrative override to cancel any stream, bypassing sender authorization.
    /// Identical behavior to `cancel_stream` but requires admin authorization instead
    /// of sender authorization. Useful for emergency interventions or dispute resolution.
    ///
    /// # Parameters
    /// - `stream_id`: Unique identifier of the stream to cancel
    ///
    /// # Authorization
    /// - Requires authorization from the contract admin (set during `init`)
    ///
    /// # Behavior
    /// Same as `cancel_stream`:
    /// 1. Validates stream is in `Active` or `Paused` state
    /// 2. Captures `cancelled_at = ledger.timestamp()`
    /// 3. Refunds `deposit_amount - accrued_at_cancelled_at` to sender
    /// 4. Persists `status = Cancelled` and `cancelled_at`
    /// 5. Emits `StreamCancelled(stream_id)`
    ///
    /// # Panics
    /// - Returns `ContractError::InvalidState` if stream is not `Active` or `Paused`
    /// - If the stream does not exist
    /// - If caller is not the admin
    /// - If token transfer fails
    ///
    /// # Events
    /// - Publishes `Cancelled(stream_id)` event on success
    ///
    /// # Usage Notes
    /// - Admin can cancel any stream regardless of sender
    /// - Use for emergency situations or dispute resolution
    /// - Sender still receives refund of unstreamed tokens
    /// - Recipient can still withdraw accrued amount
    ///
    /// # Handling of already-accrued amount
    /// - Mirrors `cancel_stream`: accrued value is never refunded to the sender.
    /// - Accrued funds stay in the contract until the recipient calls `withdraw()`.
    /// - No auto-transfer of accrued funds to the recipient occurs on admin cancel.
    pub fn cancel_stream_as_admin(env: Env, stream_id: u64) -> Result<(), ContractError> {
        get_admin(&env)?.require_auth();

        let mut stream = load_stream(&env, stream_id)?;

        Self::cancel_stream_internal(&env, &mut stream)
    }

    /// Pause a payment stream as the contract admin.
    ///
    /// Administrative override to pause any stream, bypassing sender authorization.
    /// Identical behavior to `pause_stream` but requires admin authorization instead
    /// of sender authorization.
    ///
    /// # Parameters
    /// - `stream_id`: Unique identifier of the stream to pause
    /// - `reason`: Operational reason code for the pause (see `PauseReason`)
    ///
    /// # Authorization
    /// - Requires authorization from the contract admin (set during `init`)
    ///
    /// # Panics
    /// - If the stream is not in `Active` state
    /// - If the stream does not exist
    /// - If caller is not the admin
    ///
    /// # Events
    /// - Publishes `("paused", stream_id)` → `StreamPaused { stream_id, reason }` on success
    ///
    /// # Usage Notes
    /// - Admin can pause any stream regardless of sender
    /// - Accrual continues based on time (pause doesn't stop time)
    /// - Recipient cannot withdraw while paused
    pub fn pause_stream_as_admin(
        env: Env,
        stream_id: u64,
        reason: PauseReason,
    ) -> Result<(), ContractError> {
        get_admin(&env)?.require_auth();

        let mut stream = load_stream(&env, stream_id)?;

        if stream.status == StreamStatus::Paused {
            return Err(ContractError::StreamAlreadyPaused);
        }
        if is_terminal_state(&env, &stream) {
            return Err(ContractError::StreamTerminalState);
        }
        if stream.status != StreamStatus::Active {
            return Err(ContractError::InvalidState);
        }

        stream.status = StreamStatus::Paused;
        save_stream(&env, &stream);

        env.events().publish(
            (symbol_short!("paused"), stream_id),
            StreamPaused { stream_id, reason },
        );
        Ok(())
    }

    /// Resume a paused payment stream as the contract admin.
    ///
    /// Administrative override to resume any paused stream, bypassing sender authorization.
    /// Identical behavior to `resume_stream` but requires admin authorization instead
    /// of sender authorization.
    ///
    /// # Parameters
    /// - `stream_id`: Unique identifier of the stream to resume
    ///
    /// # Authorization
    /// - Requires authorization from the contract admin (set during `init`)
    ///
    /// # Panics
    /// - If the stream is not in `Paused` state
    /// - If the stream does not exist
    /// - If caller is not the admin
    ///
    /// # Events
    /// - Publishes `Resumed(stream_id)` event on success
    ///
    /// # Usage Notes
    /// - Admin can resume any paused stream regardless of sender
    /// - After resume, recipient can immediately withdraw accrued funds
    /// - Cannot resume completed or cancelled streams (terminal states)
    pub fn resume_stream_as_admin(env: Env, stream_id: u64) -> Result<(), ContractError> {
        get_admin(&env)?.require_auth();
        let mut stream = load_stream(&env, stream_id)?;

        if stream.status == StreamStatus::Active {
            return Err(ContractError::StreamNotPaused);
        }
        if is_terminal_state(&env, &stream) {
            return Err(ContractError::StreamTerminalState);
        }
        if stream.status != StreamStatus::Paused {
            return Err(ContractError::StreamNotPaused);
        }

        stream.status = StreamStatus::Active;
        save_stream(&env, &stream);

        env.events().publish(
            (symbol_short!("resumed"), stream_id),
            StreamEvent::Resumed(stream_id),
        );
        Ok(())
    }

    /// Execute a recipient-signed delegated withdrawal (relayer support).
    ///
    /// Allows a relayer to withdraw accrued tokens on behalf of a recipient, provided the
    /// recipient has signed an authorization message. The signature covers the stream ID,
    /// destination address, nonce, and deadline, preventing replay and expiry attacks.
    ///
    /// # Signature Scheme
    ///
    /// The message that the recipient must sign is the SHA-256 hash of the following
    /// concatenated bytes:
    ///
    /// ```text
    /// "fluxora_delegated_withdraw" (UTF-8, no null terminator)
    /// || contract_address_xdr  (XDR-encoded)
    /// || destination_xdr       (XDR-encoded)
    /// || stream_id             (8 bytes, u64 big-endian)
    /// || nonce                 (8 bytes, u64 big-endian)
    /// || deadline              (8 bytes, u64 big-endian)
    /// ```
    ///
    /// The resulting 32-byte hash is verified against `signature` using the recipient's
    /// Ed25519 public key via `env.crypto().ed25519_verify`.
    ///
    /// # Parameters
    /// - `stream_id`: Unique identifier of the stream to withdraw from.
    /// - `relayer`: Address submitting the transaction (pays fees; receives no tokens).
    /// - `destination`: Address that will receive the withdrawn tokens.
    /// - `nonce`: Must equal the recipient's current nonce (monotonically increasing).
    /// - `deadline`: Ledger timestamp after which the signature is invalid.
    /// - `signature`: 64-byte Ed25519 signature produced by the recipient.
    ///
    /// # Returns
    /// - `i128`: Amount of tokens transferred to `destination` (0 if nothing to withdraw).
    ///
    /// # Authorization
    /// - Requires authorization from `relayer` (pays the transaction fee).
    /// - The recipient does **not** need to be the transaction submitter; their intent is
    ///   expressed via the off-chain signature.
    ///
    /// # Replay Protection
    /// - `nonce` must equal `get_withdraw_nonce(recipient)` exactly; any other value is rejected.
    /// - On success the nonce is incremented, invalidating the used signature.
    /// - Nonces are per-recipient and strictly monotonic (no skipping).
    ///
    /// # Expiry
    /// - `deadline` must be `>= env.ledger().timestamp()` at execution time.
    /// - Expired signatures are rejected with `SignatureDeadlineExpired`.
    ///
    /// # Destination Constraints
    /// - `destination` must not equal `env.current_contract_address()`.
    ///
    /// # CEI Ordering
    /// 1. Checks (deadline, nonce, signature, stream status, withdrawable amount).
    /// 2. Effects (increment nonce, update `withdrawn_amount`, optionally set `Completed`).
    /// 3. Interactions (token transfer, events).
    ///
    /// # Events
    /// - Publishes `("dlg_wdraw", stream_id)` → `DelegatedWithdrawal { ... }` when `amount > 0`.
    /// - Publishes `("completed", stream_id)` → `StreamEvent::StreamCompleted` if stream drains.
    ///
    /// # Errors
    /// - `SignatureDeadlineExpired` — `deadline < current ledger timestamp`.
    /// - `InvalidParams` — nonce mismatch or destination is the contract address.
    /// - `InvalidSignature` — signature does not verify against recipient's key.
    /// - `StreamNotFound` — stream does not exist.
    /// - `InvalidState` — stream is `Completed` or `Paused`.
    pub fn delegated_withdraw(
        env: Env,
        stream_id: u64,
        relayer: Address,
        destination: Address,
        nonce: u64,
        deadline: u64,
        signature: BytesN<64>,
    ) -> Result<i128, ContractError> {
        // Relayer pays the transaction fee.
        relayer.require_auth();

        // 1. Deadline check.
        if env.ledger().timestamp() > deadline {
            return Err(ContractError::SignatureDeadlineExpired);
        }

        // 2. Destination guard.
        if destination == env.current_contract_address() {
            return Err(ContractError::InvalidParams);
        }

        // 3. Load stream and check status.
        let mut stream = load_stream(&env, stream_id)?;

        if stream.status == StreamStatus::Completed {
            return Err(ContractError::InvalidState);
        }
        if stream.status == StreamStatus::Paused {
            return Err(ContractError::InvalidState);
        }

        // 4. Nonce check (replay protection).
        let current_nonce = read_withdraw_nonce(&env, &stream.recipient);
        if nonce != current_nonce {
            return Err(ContractError::InvalidParams);
        }

        // 5. Reconstruct and verify signature.
        //
        // Message = SHA-256(
        //   "fluxora_delegated_withdraw"
        //   || contract_address_xdr
        //   || destination_xdr
        //   || stream_id  (8 bytes, big-endian)
        //   || nonce      (8 bytes, big-endian)
        //   || deadline   (8 bytes, big-endian)
        // )
        let contract_addr_bytes = env.current_contract_address().to_xdr(&env);
        let dest_bytes = destination.clone().to_xdr(&env);

        let mut msg = Bytes::new(&env);
        msg.extend_from_array(b"fluxora_delegated_withdraw");
        msg.append(&contract_addr_bytes);
        msg.append(&dest_bytes);
        msg.extend_from_array(&stream_id.to_be_bytes());
        msg.extend_from_array(&nonce.to_be_bytes());
        msg.extend_from_array(&deadline.to_be_bytes());

        let msg_hash: BytesN<32> = env.crypto().sha256(&msg).into();

        // Derive the recipient's Ed25519 public key from their XDR-encoded address.
        // A Stellar G-address XDR encodes as: 4-byte type + 4-byte discriminant + 32-byte key.
        let recipient_xdr = stream.recipient.clone().to_xdr(&env);
        let xdr_len = recipient_xdr.len();
        if xdr_len < 32 {
            return Err(ContractError::InvalidSignature);
        }
        let key_offset = xdr_len - 32;
        let mut pk_arr = [0u8; 32];
        for i in 0..32u32 {
            pk_arr[i as usize] = recipient_xdr.get(key_offset + i).unwrap_or(0);
        }
        let public_key: BytesN<32> = BytesN::from_array(&env, &pk_arr);

        // ed25519_verify panics on invalid signature — map to InvalidSignature.
        // We use a try-pattern via the SDK's verify which panics on failure.
        // Callers with a bad signature will get a host trap (transaction reverted).
        env.crypto()
            .ed25519_verify(&public_key, &msg_hash.into(), &signature);

        // 6. Compute withdrawable amount.
        let accrued = Self::calculate_accrued(env.clone(), stream_id)?;
        let withdrawable = accrued - stream.withdrawn_amount;

        if withdrawable == 0 {
            // Nonce is NOT consumed when there is nothing to withdraw.
            return Ok(0);
        }

        // 7. Effects: increment nonce, update stream state (CEI).
        increment_withdraw_nonce(&env, &stream.recipient);

        stream.withdrawn_amount += withdrawable;
        let completed_now = stream.status == StreamStatus::Active
            && stream.withdrawn_amount == stream.deposit_amount;
        if completed_now {
            stream.status = StreamStatus::Completed;
        }
        save_stream(&env, &stream);

        // 8. Interactions: token transfer then events.
        push_token(&env, &destination, withdrawable)?;

        env.events().publish(
            (symbol_short!("dlg_wdraw"), stream_id),
            DelegatedWithdrawal {
                stream_id,
                recipient: stream.recipient.clone(),
                relayer,
                destination,
                amount: withdrawable,
                nonce,
            },
        );

        if completed_now {
            env.events().publish(
                (symbol_short!("completed"), stream_id),
                StreamEvent::StreamCompleted(stream_id),
            );
        }

        Ok(withdrawable)
    }

    /// Return the current delegated-withdraw nonce for a recipient.
    ///
    /// The nonce must be included in the signature payload and must match exactly
    /// when `delegated_withdraw` is called. It is incremented on every successful
    /// delegated withdrawal that moves tokens.
    ///
    /// # Parameters
    /// - `recipient`: Address to query the nonce for.
    ///
    /// # Returns
    /// - `u64`: Current nonce (0 if no delegated withdrawal has ever been executed).
    pub fn get_withdraw_nonce(env: Env, recipient: Address) -> u64 {
        read_withdraw_nonce(&env, &recipient)
    }

    /// Set or clear the **global emergency pause** flag (admin only).
    ///
    /// When `paused == true`, routine user-facing mutations revert with
    /// `"contract is globally paused"`. Admin override entrypoints
    /// (`*_as_admin`, this function) and read-only views are not blocked.
    ///
    /// # Authorization
    /// - Requires authorization from the contract admin.
    ///
    /// # Events
    /// - Publishes topic `gl_pause` with [`GlobalEmergencyPauseChanged`] data.
    pub fn set_global_emergency_paused(env: Env, paused: bool) {
        let admin = get_admin(&env).unwrap();
        admin.require_auth();

        env.storage()
            .instance()
            .set(&DataKey::GlobalEmergencyPaused, &paused);
        bump_instance_ttl(&env);

        env.events().publish(
            (symbol_short!("gl_pause"),),
            GlobalEmergencyPauseChanged { paused },
        );
    }

    /// Explicitly clear the **global emergency pause** and restore normal contract behaviour.
    ///
    /// This is the dedicated, unambiguous counterpart to `set_global_emergency_paused(true)`.
    /// Calling it is equivalent to `set_global_emergency_paused(false)` but emits a distinct
    /// `GlobalResumed` event so that incident-response tooling and indexers can distinguish a
    /// deliberate post-incident resume from a routine toggle.
    ///
    /// # Authorization
    /// - Requires authorization from the contract admin.
    ///
    /// # Errors
    /// - Returns `ContractError::InvalidState` if the contract is **not** currently in
    ///   emergency pause (prevents spurious resume events and double-resume confusion).
    ///
    /// # State Changes
    /// - Clears `DataKey::GlobalEmergencyPaused` (sets it to `false`).
    /// - All user-facing mutations that were blocked by the emergency pause are immediately
    ///   re-enabled: `create_stream`, `create_streams`, `withdraw`, `withdraw_to`,
    ///   `batch_withdraw`, `cancel_stream`, `update_rate_per_second`,
    ///   `shorten_stream_end_time`, `extend_stream_end_time`.
    ///
    /// # Events
    /// - Publishes topic `gl_resume` with [`GlobalResumed`] data containing the ledger
    ///   timestamp at which the resume occurred.
    ///
    /// # Post-incident checklist
    /// After calling `global_resume`, operators should:
    /// 1. Verify `get_global_emergency_paused()` returns `false`.
    /// 2. Confirm the `gl_resume` event appears in the transaction record.
    /// 3. Run smoke-test transactions (e.g. a small `create_stream`) to confirm normal operation.
    /// 4. Review any streams that were paused or cancelled during the incident window.
    /// 5. Communicate the all-clear to protocol users and downstream integrators.
    pub fn global_resume(env: Env) -> Result<(), ContractError> {
        let admin = get_admin(&env)?;
        admin.require_auth();

        if !is_global_emergency_paused(&env) {
            return Err(ContractError::InvalidState);
        }

        env.storage()
            .instance()
            .set(&DataKey::GlobalEmergencyPaused, &false);
        bump_instance_ttl(&env);

        env.events().publish(
            (symbol_short!("gl_resume"),),
            GlobalResumed {
                resumed_at: env.ledger().timestamp(),
            },
        );

        Ok(())
    }

    /// Toggle the **contract pause** flag to prevent/restore stream creation.
    ///
    /// When `paused == true`, `create_stream` and `create_streams` revert with
    /// `ContractError::ContractPaused`. All other operations are unaffected.
    ///
    /// This is distinct from global pause, which blocks all operations.
    ///
    /// # Authorization
    /// - Requires authorization from the contract admin.
    ///
    /// # Events
    /// - Publishes topic `ct_pause` with [`ContractPauseChanged`] data.
    pub fn set_contract_paused(env: Env, paused: bool) -> Result<(), ContractError> {
        get_admin(&env)?.require_auth();

        env.storage()
            .instance()
            .set(&DataKey::CreationPaused, &paused);
        bump_instance_ttl(&env);

        env.events().publish(
            (soroban_sdk::Symbol::new(&env, "paused_ctl"),),
            ContractPauseChanged { paused },
        );

        Ok(())
    }

    /// Globally pause the protocol to block new stream creation.
    ///
    /// # Authorization
    /// - Requires authorization from the contract admin.
    ///
    /// # Idempotency
    /// - If the protocol is already paused, this is a no-op and returns Ok(()) silently.
    /// - No storage changes or events are emitted on idempotent calls.
    ///
    /// # State Changes (when not already paused)
    /// - Sets `DataKey::GlobalEmergencyPaused` to true
    /// - Stores `reason` (or empty string if None) in `DataKey::GlobalPauseReason`
    /// - Stores current ledger timestamp in `DataKey::GlobalPauseTimestamp`
    /// - Stores admin address in `DataKey::GlobalPauseAdmin`
    ///
    /// # Events
    /// - Emits `ProtocolPaused` event with reason and timestamp (only on actual pause)
    pub fn pause_protocol(
        env: Env,
        admin: Address,
        reason: Option<soroban_sdk::String>,
    ) -> Result<(), ContractError> {
        admin.require_auth();

        // Verify caller is the stored admin
        let stored_admin = get_admin(&env)?;
        if admin != stored_admin {
            return Err(ContractError::Unauthorized);
        }

        // Idempotent: if already paused, return silently
        if is_protocol_paused(&env) {
            // Idempotent: re-pausing is a no-op
            return Ok(());
        }

        // Set the global emergency pause flag
        env.storage()
            .instance()
            .set(&DataKey::GlobalEmergencyPaused, &true);

        // Store audit trail information
        let reason_str = reason.unwrap_or_else(|| soroban_sdk::String::from_str(&env, ""));
        env.storage()
            .instance()
            .set(&DataKey::GlobalPauseReason, &reason_str);

        let now = env.ledger().timestamp();
        env.storage()
            .instance()
            .set(&DataKey::GlobalPauseTimestamp, &now);
        env.storage()
            .instance()
            .set(&DataKey::GlobalPauseAdmin, &admin);

        bump_instance_ttl(&env);

        // Emit ProtocolPaused event AFTER storage is written
        env.events().publish(
            (symbol_short!("pr_pause"), admin.clone()),
            ProtocolPaused {
                reason: reason_str,
                paused_at: now,
            },
        );

        Ok(())
    }

    /// Globally resume the protocol to allow new stream creation.
    ///
    /// # Authorization
    /// - Requires authorization from the contract admin.
    ///
    /// # Idempotency
    /// - If the protocol is not currently paused, this is a no-op and returns Ok(()) silently.
    /// - No storage changes or events are emitted on idempotent calls.
    ///
    /// # State Changes (when currently paused)
    /// - Clears `DataKey::GlobalEmergencyPaused` (sets to false)
    /// - Clears `DataKey::GlobalPauseReason`
    /// - Clears `DataKey::GlobalPauseTimestamp`
    /// - Clears `DataKey::GlobalPauseAdmin`
    ///
    /// # Events
    /// - Emits `ProtocolResumed` event with timestamp (only on actual resume)
    pub fn resume_protocol(env: Env, admin: Address) -> Result<(), ContractError> {
        admin.require_auth();

        // Verify caller is the stored admin
        let stored_admin = get_admin(&env)?;
        if admin != stored_admin {
            return Err(ContractError::Unauthorized);
        }

        // Idempotent: if not paused, return silently
        if !is_protocol_paused(&env) {
            // Idempotent: resuming when not paused is a no-op
            return Ok(());
        }

        // Clear all pause-related storage
        env.storage()
            .instance()
            .set(&DataKey::GlobalEmergencyPaused, &false);
        env.storage().instance().remove(&DataKey::GlobalPauseReason);
        env.storage()
            .instance()
            .remove(&DataKey::GlobalPauseTimestamp);
        env.storage().instance().remove(&DataKey::GlobalPauseAdmin);

        bump_instance_ttl(&env);

        // Emit ProtocolResumed event
        let now = env.ledger().timestamp();
        env.events().publish(
            (symbol_short!("pr_resume"), admin),
            ProtocolResumed { resumed_at: now },
        );

        Ok(())
    }

    /// Query whether the protocol is currently paused.
    ///
    /// # Authorization
    /// - None required. Anyone can call this.
    ///
    /// # Returns
    /// - `true` if the protocol is paused (creation blocked)
    /// - `false` if the protocol is active (creation allowed)
    pub fn is_paused(env: Env) -> bool {
        is_protocol_paused(&env)
    }

    /// Query detailed pause information including reason, timestamp, and admin.
    ///
    /// # Authorization
    /// - None required. Anyone can call this.
    ///
    /// # Returns
    /// - `PauseInfo` struct with `is_paused`, `reason`, `paused_at`, `paused_by` fields.
    /// - All optional fields are `None` when not paused.
    pub fn get_pause_info(env: Env) -> PauseInfo {
        let is_paused = is_protocol_paused(&env);
        if is_paused {
            PauseInfo {
                is_paused: true,
                reason: get_pause_reason(&env),
                paused_at: get_pause_timestamp(&env),
                paused_by: get_pause_admin(&env),
            }
        } else {
            PauseInfo {
                is_paused: false,
                reason: None,
                paused_at: None,
                paused_by: None,
            }
        }
    }
}

#[cfg(test)]
mod test;
#[cfg(test)]
mod test_issue_39;
#[cfg(test)]
mod test_withdrawable_props;
