#![no_std]

mod accrual;

use soroban_sdk::{
    contract, contractimpl, contracttype, panic_with_error, symbol_short, token, Address, Env,
    Symbol,
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
// Contract version
// ---------------------------------------------------------------------------

/// Compile-time contract version number.
///
/// Increment this constant whenever a breaking change is deployed so that
/// frontends and scripts can detect which version is running on-chain.
pub const CONTRACT_VERSION: u32 = 1;

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

/// Global configuration for the Fluxora protocol.
#[contracttype]
#[derive(Clone, Debug)]
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

/// Per-stream result for `batch_withdraw`.
#[contracttype]
#[derive(Clone, Debug)]
pub struct BatchWithdrawResult {
    pub stream_id: u64,
    pub amount: i128,
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

#[contracttype]
#[derive(Clone, Debug)]
pub struct StreamEndShortened {
    pub stream_id: u64,
    pub old_end_time: u64,
    pub new_end_time: u64,
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
}

#[contracttype]
#[derive(Clone, Debug)]
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
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct CreateStreamParams {
    pub recipient: Address,
    pub deposit_amount: i128,
    pub rate_per_second: i128,
    pub start_time: u64,
    pub cliff_time: u64,
    pub end_time: u64,
}

/// Namespace for all contract storage keys.
#[contracttype]
pub enum DataKey {
    Config,                    // Instance storage for global settings (admin/token).
    NextStreamId,              // Instance storage for the auto-incrementing ID counter.
    Stream(u64),               // Persistent storage for individual stream data (O(1) lookup).
    RecipientStreams(Address), // Persistent storage for recipient stream index (sorted by stream_id).
    /// Emergency pause flag (bool). Appended to avoid shifting existing key discriminants.
    GlobalPaused,
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

fn get_config(env: &Env) -> Config {
    bump_instance_ttl(env);
    env.storage()
        .instance()
        .get(&DataKey::Config)
        .expect("contract not initialised: missing config")
}

fn get_token(env: &Env) -> Address {
    get_config(env).token
}

fn get_admin(env: &Env) -> Address {
    get_config(env).admin
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

fn read_global_paused(env: &Env) -> bool {
    bump_instance_ttl(env);
    env.storage()
        .instance()
        .get(&DataKey::GlobalPaused)
        .unwrap_or(false)
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

fn save_stream(env: &Env, stream: &Stream) {
    let key = DataKey::Stream(stream.stream_id);
    env.storage().persistent().set(&key, stream);

    // Extend TTL on stream save to ensure persistence
    env.storage().persistent().extend_ttl(
        &key,
        PERSISTENT_LIFETIME_THRESHOLD,
        PERSISTENT_BUMP_AMOUNT,
    );
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
    let mut insert_pos: u32 = 0;
    for (i, id) in streams.iter().enumerate() {
        if id > stream_id {
            insert_pos = i as u32;
            break;
        }
        insert_pos = (i + 1) as u32;
    }

    streams.insert(insert_pos, stream_id);
    save_recipient_streams(env, recipient, &streams);
}

/// Remove a stream ID from a recipient's index.
fn remove_stream_from_recipient_index(env: &Env, recipient: &Address, stream_id: u64) {
    let mut streams = load_recipient_streams(env, recipient);

    // Find and remove the stream_id
    let mut found_index = None;
    for (i, id) in streams.iter().enumerate() {
        if id == stream_id {
            found_index = Some(i as u32);
            break;
        }
    }

    if let Some(idx) = found_index {
        streams.remove(idx);
        save_recipient_streams(env, recipient, &streams);
    }
}

// ---------------------------------------------------------------------------
// Token transfer helpers
// ---------------------------------------------------------------------------

/// Pull tokens from an external address to the contract.
///
/// Centralizes all token transfers INTO the contract for security review.
/// Used when creating streams to pull deposit from sender.
///
/// # Parameters
/// - `env`: Contract environment
/// - `from`: Address to transfer tokens from (must have approved contract)
/// - `amount`: Amount of tokens to transfer
///
/// # Panics
/// - If token transfer fails (insufficient balance or allowance)
fn pull_token(env: &Env, from: &Address, amount: i128) {
    let token_client = token::Client::new(env, &get_token(env));
    token_client.transfer(from, &env.current_contract_address(), &amount);
}

/// Push tokens from the contract to an external address.
///
/// Centralizes all token transfers OUT OF the contract for security review.
/// Used for withdrawals (to recipient) and refunds (to sender on cancel).
///
/// # Parameters
/// - `env`: Contract environment
/// - `to`: Address to transfer tokens to
/// - `amount`: Amount of tokens to transfer
///
/// # Panics
/// - If token transfer fails (insufficient contract balance, should not happen)
fn push_token(env: &Env, to: &Address, amount: i128) {
    let token_client = token::Client::new(env, &get_token(env));
    token_client.transfer(&env.current_contract_address(), to, &amount);
}

// ---------------------------------------------------------------------------
// Internal Helpers
// ---------------------------------------------------------------------------

impl FluxoraStream {
    #[allow(clippy::too_many_arguments)]
    fn validate_stream_params(
        env: &Env,
        sender: &Address,
        recipient: &Address,
        deposit_amount: i128,
        rate_per_second: i128,
        current_ledger_timestamp: u64,
        start_time: u64,
        cliff_time: u64,
        end_time: u64,
    ) {
        // Validate positive amounts (#35)
        assert!(deposit_amount > 0, "deposit_amount must be positive");
        assert!(rate_per_second > 0, "rate_per_second must be positive");

        // Validate sender != recipient (#35)
        assert!(
            sender != recipient,
            "sender and recipient must be different"
        );

        // Validate time constraints
        assert!(start_time < end_time, "start_time must be before end_time");
        if start_time < current_ledger_timestamp {
            panic_with_error!(env, ContractError::StartTimeInPast);
        }
        assert!(
            cliff_time >= start_time && cliff_time <= end_time,
            "cliff_time must be within [start_time, end_time]"
        );

        // Validate deposit covers total streamable amount (#34)
        let duration = (end_time - start_time) as i128;
        let total_streamable = rate_per_second
            .checked_mul(duration)
            .expect("overflow calculating total streamable amount");
        assert!(
            deposit_amount >= total_streamable,
            "deposit_amount must cover total streamable amount (rate * duration)"
        );
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
    ) -> u64 {
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
        };

        save_stream(env, &stream);

        // Add stream to recipient's index (maintains sorted order by stream_id)
        add_stream_to_recipient_index(env, &recipient, stream_id);

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
            },
        );

        stream_id
    }
}

// ---------------------------------------------------------------------------
// Contract Implementation
// ---------------------------------------------------------------------------

#[contract]
pub struct FluxoraStream;

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
    ///
    /// # Storage
    /// - Stores `Config { token, admin }` in instance storage under `DataKey::Config`
    /// - Initializes `NextStreamId` counter to 0 for stream ID generation
    /// - Extends TTL to prevent premature expiration (17280 ledgers threshold, 120960 max)
    ///
    /// # Panics
    /// - If called more than once (contract already initialized)
    ///
    /// # Security
    /// - Re-initialization is prevented to ensure immutable token and admin configuration
    /// - No authorization required for initial setup (deployer calls this once)
    pub fn init(env: Env, token: Address, admin: Address) {
        if env.storage().instance().has(&DataKey::Config) {
            panic!("already initialised");
        }
        let config = Config { token, admin };
        env.storage().instance().set(&DataKey::Config, &config);
        env.storage().instance().set(&DataKey::NextStreamId, &0u64);

        // Ensure instance storage (Config / NextStreamId) doesn't expire quickly
        bump_instance_ttl(&env);
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
    ) -> u64 {
        sender.require_auth();
        if read_global_paused(&env) {
            panic_with_error!(&env, ContractError::ContractPaused);
        }

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
        );

        // Transfer tokens from sender to this contract (#36)
        // If transfer fails (insufficient balance/allowance), this will panic
        // and no state will be persisted (atomic transaction)
        pull_token(&env, &sender, deposit_amount);

        // Only allocate stream id and persist state AFTER successful transfer
        Self::persist_new_stream(
            &env,
            sender,
            recipient,
            deposit_amount,
            rate_per_second,
            start_time,
            cliff_time,
            end_time,
        )
    }

    /// Create multiple payment streams in a single transaction.
    ///
    /// Optimizes gas usage by verifying authorization once and doing a single bulk
    /// token transfer for all streams, executing the creations atomically.
    ///
    /// # Parameters
    /// - `sender`: Address funding all streams in the batch
    /// - `streams`: Vector of stream configuration parameters
    ///
    /// # Returns
    /// - `Vec<u64>`: Vector of unique stream identifiers for the newly created streams
    ///
    /// # Authorization
    /// - Requires authorization from the sender address exactly once for the entire batch.
    pub fn create_streams(
        env: Env,
        sender: Address,
        streams: soroban_sdk::Vec<CreateStreamParams>,
    ) -> soroban_sdk::Vec<u64> {
        sender.require_auth();
        if read_global_paused(&env) {
            panic_with_error!(&env, ContractError::ContractPaused);
        }

        let mut total_deposit: i128 = 0;

        // First pass: validate all streams and calculate total deposit required
        for params in streams.iter() {
            Self::validate_stream_params(
                &env,
                &sender,
                &params.recipient,
                params.deposit_amount,
                params.rate_per_second,
                env.ledger().timestamp(),
                params.start_time,
                params.cliff_time,
                params.end_time,
            );
            total_deposit = total_deposit
                .checked_add(params.deposit_amount)
                .expect("overflow calculating total batch deposit");
        }

        // Bulk transfer tokens from sender to this contract atomically to save gas
        if total_deposit > 0 {
            let token_client = token::Client::new(&env, &get_token(&env));
            token_client.transfer(&sender, &env.current_contract_address(), &total_deposit);
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
            );
            created_ids.push_back(stream_id);
        }

        created_ids
    }

    /// Pause an active payment stream.
    ///
    /// Temporarily halts withdrawals from the stream while preserving accrual calculations.
    /// The stream can be resumed later by the sender or admin. Accrual continues based on
    /// time elapsed, but the recipient cannot withdraw while paused.
    ///
    /// # Parameters
    /// - `stream_id`: Unique identifier of the stream to pause
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
    /// - Publishes `Paused(stream_id)` event on success
    ///
    /// # Usage Notes
    /// - Pausing does not affect accrual calculations (time-based)
    /// - Recipient cannot withdraw while stream is paused
    /// - Stream can be cancelled while paused
    /// - Use `resume_stream` to reactivate withdrawals
    pub fn pause_stream(env: Env, stream_id: u64) -> Result<(), ContractError> {
        let mut stream = match load_stream(&env, stream_id) {
            Ok(stream) => stream,
            Err(err) => panic_with_error!(&env, err),
        };

        Self::require_stream_sender(&stream.sender);

        if stream.status == StreamStatus::Paused {
            panic!("stream is already paused");
        }

        assert!(
            stream.status == StreamStatus::Active,
            "stream must be active to pause"
        );

        stream.status = StreamStatus::Paused;
        save_stream(&env, &stream);

        env.events().publish(
            (symbol_short!("paused"), stream_id),
            StreamEvent::Paused(stream_id),
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
        let mut stream = load_stream(&env, stream_id).unwrap_or_else(|err| {
            panic_with_error!(&env, err);
        });
        Self::require_stream_sender(&stream.sender);

        match stream.status {
            StreamStatus::Active => panic!("stream is active, not paused"),
            StreamStatus::Completed => panic!("stream is completed"),
            StreamStatus::Cancelled => panic!("stream is cancelled"),
            StreamStatus::Paused => {}
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
    /// 2. Calculates accrued amount: `min((now - start_time) × rate, deposit_amount)`
    /// 3. Calculates refund: `deposit_amount - accrued`
    /// 4. Transfers refund to sender (if > 0)
    /// 5. Sets stream status to `Cancelled`
    /// 6. Accrued but not withdrawn amount remains for recipient
    ///
    /// # Returns
    /// - Implicitly returns via state change and token transfer
    ///
    /// # Panics
    /// - If stream is not `Active` or `Paused` (already completed or cancelled)
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
        let mut stream = load_stream(&env, stream_id)?;
        Self::require_stream_sender(&stream.sender);
        Self::require_cancellable_status(&env, stream.status);

        let accrued = Self::calculate_accrued(env.clone(), stream_id)?;
        let unstreamed = stream.deposit_amount - accrued;

        // CEI: update state before external token transfer to reduce reentrancy risk.
        // Assumption: the token contract does not reenter this contract.
        stream.status = StreamStatus::Cancelled;
        stream.cancelled_at = Some(env.ledger().timestamp());
        save_stream(&env, &stream);

        if unstreamed > 0 {
            push_token(&env, &stream.sender, unstreamed);
        }

        env.events().publish(
            (symbol_short!("cancelled"), stream_id),
            StreamEvent::StreamCancelled(stream_id),
        );
        Ok(())
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
    /// - If `accrued == withdrawn_amount` (nothing to withdraw), returns 0 immediately
    /// - No token transfer occurs, no state change, no event published
    /// - This is idempotent: safe to call multiple times without side effects
    /// - Occurs before cliff time or when all accrued funds already withdrawn
    /// - Frontends can call withdraw without pre-checking balance
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
    /// - Sets status to `Completed` if all deposited tokens are withdrawn
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
    /// - For cancelled streams, only the accrued amount (not refunded) can be withdrawn
    ///
    /// # Examples
    /// - Stream: 1000 tokens over 1000 seconds (1 token/sec)
    /// - At t=0 (before cliff): withdraw() returns 0 (no transfer)
    /// - At t=300: withdraw() returns 300 tokens
    /// - At t=300 (again): withdraw() returns 0 (already withdrawn)
    /// - At t=800: withdraw() returns 500 tokens (800 - 300 already withdrawn)
    /// - At t=1000: withdraw() returns 200 tokens, status → Completed
    pub fn withdraw(env: Env, stream_id: u64) -> Result<i128, ContractError> {
        let mut stream = load_stream(&env, stream_id)?;

        // Enforce recipient-only authorization
        stream.recipient.require_auth();

        assert!(
            stream.status != StreamStatus::Completed,
            "stream already completed"
        );

        assert!(
            stream.status != StreamStatus::Paused,
            "cannot withdraw from paused stream"
        );

        let accrued = Self::calculate_accrued(env.clone(), stream_id)?;
        let withdrawable = accrued - stream.withdrawn_amount;

        // Handle zero withdrawable: return 0 without transfer or state change (idempotent).
        // This occurs before cliff or when all accrued funds have been withdrawn.
        // Frontends can safely call withdraw without checking balance first.
        if withdrawable == 0 {
            return Ok(0);
        }

        // CEI: update state before external token transfer to reduce reentrancy risk.
        // Assumption: the token contract does not reenter this contract.
        stream.withdrawn_amount += withdrawable;
        let completed_now = stream.withdrawn_amount == stream.deposit_amount;
        if completed_now {
            stream.status = StreamStatus::Completed;
        }
        save_stream(&env, &stream);

        push_token(&env, &stream.recipient, withdrawable);

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
    /// Same accounting as `withdraw`, but transfers tokens to `destination` instead of
    /// the stream's recipient. Use for wallet migration or custody workflows. The caller
    /// must still be the stream's recipient (recipient-authorized).
    ///
    /// # Parameters
    /// - `stream_id`: Unique identifier of the stream to withdraw from
    /// - `destination`: Address to receive the withdrawn tokens (must not be the contract)
    ///
    /// # Returns
    /// - `i128`: The amount of tokens transferred to the destination (0 if nothing to withdraw)
    ///
    /// # Authorization
    /// - Requires authorization from the stream's recipient (only recipient can withdraw)
    ///
    /// # Validation
    /// - `destination` must not be the contract address (tokens must leave the contract)
    ///
    /// # Panics
    /// - If the stream is `Completed` or `Paused`
    /// - If the stream does not exist or caller is not the recipient
    /// - If `destination` is the contract address
    pub fn withdraw_to(
        env: Env,
        stream_id: u64,
        destination: Address,
    ) -> Result<i128, ContractError> {
        let mut stream = load_stream(&env, stream_id)?;

        stream.recipient.require_auth();

        assert!(
            destination != env.current_contract_address(),
            "destination must not be the contract"
        );

        assert!(
            stream.status != StreamStatus::Completed,
            "stream already completed"
        );

        assert!(
            stream.status != StreamStatus::Paused,
            "cannot withdraw from paused stream"
        );

        let accrued = Self::calculate_accrued(env.clone(), stream_id)?;
        let withdrawable = accrued - stream.withdrawn_amount;

        if withdrawable == 0 {
            return Ok(0);
        }

        stream.withdrawn_amount += withdrawable;
        let completed_now = stream.withdrawn_amount == stream.deposit_amount;
        if completed_now {
            stream.status = StreamStatus::Completed;
        }
        save_stream(&env, &stream);

        push_token(&env, &destination, withdrawable);

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

    /// Withdraw accrued tokens from multiple streams in one call (recipient-only).
    ///
    /// The caller must be the recipient of every stream in `stream_ids`. Each stream
    /// is processed in order: same validation and accounting as `withdraw`. Events
    /// are emitted per stream. The operation is atomic: if any stream fails
    /// (e.g. not found, not recipient's, completed, or paused), the entire call panics
    /// and no state changes or transfers occur.
    ///
    /// # Parameters
    /// - `recipient`: Address that must authorize and must be the recipient of all streams
    /// - `stream_ids`: Stream IDs to withdraw from (can contain duplicates; each processed once)
    ///
    /// # Returns
    /// - `Vec<BatchWithdrawResult>`: Per-stream (stream_id, amount) for each withdrawal (amount may be 0)
    ///
    /// # Authorization
    /// - Requires authorization from `recipient` once for the entire batch
    ///
    /// # Atomicity
    /// - All streams are processed in order. Any panic (stream not found, wrong recipient,
    ///   completed, paused) reverts the whole transaction.
    pub fn batch_withdraw(
        env: Env,
        recipient: Address,
        stream_ids: soroban_sdk::Vec<u64>,
    ) -> Result<soroban_sdk::Vec<BatchWithdrawResult>, ContractError> {
        recipient.require_auth();

        let mut results = soroban_sdk::Vec::new(&env);

        for stream_id in stream_ids.iter() {
            let mut stream = load_stream(&env, stream_id)?;

            assert!(
                stream.recipient == recipient,
                "stream recipient must match authorized recipient"
            );

            assert!(
                stream.status != StreamStatus::Paused,
                "cannot withdraw from paused stream"
            );

            let withdrawable = if stream.status == StreamStatus::Completed {
                0
            } else {
                let accrued = Self::calculate_accrued(env.clone(), stream_id)?;
                (accrued - stream.withdrawn_amount).max(0)
            };

            if withdrawable > 0 {
                stream.withdrawn_amount += withdrawable;
                let completed_now = stream.withdrawn_amount == stream.deposit_amount;
                if completed_now {
                    stream.status = StreamStatus::Completed;
                }
                save_stream(&env, &stream);

                push_token(&env, &stream.recipient, withdrawable);

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

        Ok(accrual::calculate_accrued_amount(
            stream.start_time,
            stream.cliff_time,
            stream.end_time,
            stream.rate_per_second,
            stream.deposit_amount,
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
    /// - `i128`: The amount currently available to withdraw. Returns `0` if the stream
    ///   is `Paused`, `Completed`, or before the cliff time.
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
        let withdrawable = accrued - stream.withdrawn_amount;

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

        let accrued = accrual::calculate_accrued_amount(
            stream.start_time,
            stream.cliff_time,
            stream.end_time,
            stream.rate_per_second,
            stream.deposit_amount,
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
    pub fn get_config(env: Env) -> Config {
        get_config(&env)
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
    /// # Examples
    /// - Rotate to a new admin key: `set_admin(env, new_admin_address)`
    /// - Transfer admin to a multisig: `set_admin(env, multisig_address)`
    pub fn set_admin(env: Env, new_admin: Address) {
        let mut config = get_config(&env);
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
            .publish((Symbol::new(&env, "AdminUpdated"),), (old_admin, new_admin));
    }

    /// Set global pause; create_stream/create_streams panic_with_error(ContractPaused) while true.
    pub fn set_contract_paused(env: Env, paused: bool) {
        get_admin(&env).require_auth();
        env.storage()
            .instance()
            .set(&DataKey::GlobalPaused, &paused);
        bump_instance_ttl(&env);
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
    pub fn get_stream_state(env: Env, stream_id: u64) -> Stream {
        load_stream(&env, stream_id).expect("stream not found")
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
        let mut stream = load_stream(&env, stream_id)?;

        // Only the original sender can update the rate.
        Self::require_stream_sender(&stream.sender);

        // Only mutable (non-terminal) streams can be updated.
        assert!(
            stream.status == StreamStatus::Active || stream.status == StreamStatus::Paused,
            "can only update active or paused streams"
        );

        assert!(new_rate_per_second > 0, "new rate must be positive");

        let old_rate = stream.rate_per_second;
        // Forward-only semantics: disallow decreases.
        assert!(
            new_rate_per_second > old_rate,
            "new rate must be greater than current rate"
        );

        // Validate that the existing deposit still covers the new total streamable amount.
        let duration = (stream.end_time - stream.start_time) as i128;
        let total_streamable = new_rate_per_second
            .checked_mul(duration)
            .expect("overflow calculating total streamable amount for new rate");
        assert!(
            stream.deposit_amount >= total_streamable,
            "deposit_amount must cover total streamable amount for new rate"
        );

        stream.rate_per_second = new_rate_per_second;
        save_stream(&env, &stream);

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
        let mut stream = load_stream(&env, stream_id)?;

        // Only the original sender can modify the schedule.
        Self::require_stream_sender(&stream.sender);

        // Only non-terminal streams may be shortened.
        assert!(
            stream.status == StreamStatus::Active || stream.status == StreamStatus::Paused,
            "can only shorten active or paused streams"
        );

        let now = env.ledger().timestamp();

        // New end time must be in the future relative to the current ledger timestamp.
        assert!(
            new_end_time >= now,
            "new end_time must be >= current ledger timestamp"
        );

        // Must strictly shorten the schedule.
        assert!(
            new_end_time < stream.end_time,
            "new end_time must be before existing end_time"
        );

        // Preserve basic schedule invariants.
        assert!(
            new_end_time > stream.start_time,
            "new end_time must be after start_time"
        );
        assert!(
            new_end_time >= stream.cliff_time,
            "new end_time must be >= cliff_time"
        );

        // Compute new maximum streamable amount under the shortened schedule.
        let new_duration = (new_end_time - stream.start_time) as i128;
        let new_max_streamable = stream
            .rate_per_second
            .checked_mul(new_duration)
            .expect("overflow calculating total streamable amount for shortened schedule");

        // Deposit must still be sufficient to cover the shortened schedule (by construction
        // this should hold given the original validation, but we keep an explicit assert).
        assert!(
            new_max_streamable <= stream.deposit_amount,
            "shortened schedule must not increase total streamable amount"
        );

        let old_end_time = stream.end_time;
        let old_deposit = stream.deposit_amount;
        let refund_amount = old_deposit - new_max_streamable;

        stream.end_time = new_end_time;
        stream.deposit_amount = new_max_streamable;
        save_stream(&env, &stream);

        if refund_amount > 0 {
            push_token(&env, &stream.sender, refund_amount);
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
        let mut stream = load_stream(&env, stream_id)?;

        // Only the original sender can modify the schedule.
        Self::require_stream_sender(&stream.sender);

        // Only non-terminal streams may be extended.
        assert!(
            stream.status == StreamStatus::Active || stream.status == StreamStatus::Paused,
            "can only extend active or paused streams"
        );

        let now = env.ledger().timestamp();

        // Must move end_time forward in time.
        assert!(
            new_end_time > stream.end_time,
            "new end_time must be after existing end_time"
        );

        // Keep basic schedule invariants intact.
        assert!(
            new_end_time > stream.start_time,
            "new end_time must be after start_time"
        );
        assert!(
            new_end_time >= stream.cliff_time,
            "new end_time must be >= cliff_time"
        );
        assert!(
            new_end_time >= now,
            "new end_time must be >= current ledger timestamp"
        );

        // Ensure existing deposit still covers the extended schedule at the current rate.
        let new_duration = (new_end_time - stream.start_time) as i128;
        let new_total_streamable = stream
            .rate_per_second
            .checked_mul(new_duration)
            .expect("overflow calculating total streamable amount for extended schedule");

        assert!(
            new_total_streamable <= stream.deposit_amount,
            "deposit_amount must cover total streamable amount for extended schedule"
        );

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
    /// - `funder`: Address providing the additional tokens. This may be the original
    ///   sender, the contract admin, or any other authorized treasury address.
    /// - `amount`: Additional amount of tokens to lock into the stream (must be > 0).
    ///
    /// # Authorization
    /// - Requires authorization from `funder`.
    /// - No special relationship between `funder` and the original `sender` is enforced
    ///   by the contract; protocol operators should constrain who can call this entrypoint
    ///   at the application layer (e.g. only treasury multisig).
    ///
    /// # Behaviour
    /// - Pulls `amount` tokens from `funder` into the contract.
    /// - Increases `deposit_amount` by `amount` (with overflow protection).
    /// - Does **not** modify `rate_per_second` or any timing fields.
    /// - Preserves all invariants: `deposit_amount` remains greater than or equal to
    ///   the maximum streamable amount under the current schedule.
    ///
    /// # Restrictions
    /// - Only streams in `Active` or `Paused` status can be topped up.
    /// - `amount` must be strictly positive.
    ///
    /// # Events
    /// - Emits a `top_up` event with `StreamToppedUp` payload on success.
    pub fn top_up_stream(env: Env, stream_id: u64, funder: Address, amount: i128) {
        funder.require_auth();

        assert!(amount > 0, "top up amount must be positive");

        let mut stream = match load_stream(&env, stream_id) {
            Ok(stream) => stream,
            Err(err) => panic_with_error!(&env, err),
        };

        assert!(
            stream.status == StreamStatus::Active || stream.status == StreamStatus::Paused,
            "can only top up active or paused streams"
        );

        // CEI: update and persist state BEFORE the external token transfer to reduce
        // reentrancy risk. This mirrors the pattern used in cancel_stream and withdraw.
        // Increase deposit_amount with overflow protection.
        stream.deposit_amount = stream
            .deposit_amount
            .checked_add(amount)
            .expect("overflow increasing stream deposit_amount");

        save_stream(&env, &stream);

        // External token pull happens AFTER state is persisted (CEI-compliant).
        pull_token(&env, &funder, amount);

        env.events().publish(
            (symbol_short!("top_up"), stream_id),
            StreamToppedUp {
                stream_id,
                top_up_amount: amount,
                new_deposit_amount: stream.deposit_amount,
            },
        );
    }

    /// Return the contract version number.
    ///
    /// Reads the compile-time `CONTRACT_VERSION` constant — no storage access required.
    /// Frontends and deployment scripts can call this to confirm which version of the
    /// contract is currently deployed before interacting with it.
    ///
    /// # Returns
    /// - `u32`: The current contract version (currently `1`)
    ///
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
    /// - Publishes `closed(stream_id)` with `StreamEvent::StreamClosed(stream_id)` before removal
    ///
    /// # Operational guidance
    /// - Callable by anyone; no authorization required (permissionless cleanup).
    /// - Indexers and UIs should treat closed stream IDs as non-existent.
    /// - Do not close streams that might still need historical data for accounting.
    pub fn close_completed_stream(env: Env, stream_id: u64) -> Result<(), ContractError> {
        let stream = load_stream(&env, stream_id)?;

        assert!(
            stream.status == StreamStatus::Completed,
            "can only close completed streams"
        );

        env.events().publish(
            (symbol_short!("closed"), stream_id),
            StreamEvent::StreamClosed(stream_id),
        );

        // Remove stream from recipient's index before deleting the stream
        remove_stream_from_recipient_index(&env, &stream.recipient, stream_id);
        remove_stream(&env, stream_id);

        Ok(())
    }

    /// Return the contract version number.
    ///
    /// Reads the compile-time `CONTRACT_VERSION` constant — no storage access required.
    /// Frontends and deployment scripts can call this to confirm which version of the
    /// contract is currently deployed before interacting with it.
    ///
    /// # Returns
    /// - `u32`: The current contract version (currently `1`)
    ///
    /// # Usage Notes
    /// - This is a view function (read-only, no state changes)
    /// - No authorization required (public information)
    /// - Version is a compile-time constant; calling this costs minimal gas
    /// - Increment `CONTRACT_VERSION` and redeploy when introducing breaking changes
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
    ///
    /// Returns the count of streams where the recipient is the stream's recipient address.
    /// This is a convenience function that avoids fetching the full vector when only
    /// the count is needed.
    ///
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

    /// Internal helper to require authorization from the stream sender.
    ///
    /// Admin override paths are handled by dedicated `*_as_admin` entrypoints.
    fn require_stream_sender(sender: &Address) {
        sender.require_auth();
    }

    fn require_cancellable_status(env: &Env, status: StreamStatus) {
        if status != StreamStatus::Active && status != StreamStatus::Paused {
            panic_with_error!(env, ContractError::InvalidState);
        }
    }
}

#[contractimpl]
impl FluxoraStream {
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
    /// 2. Calculates accrued amount based on time elapsed
    /// 3. Refunds unstreamed tokens to sender
    /// 4. Sets stream status to `Cancelled`
    /// 5. Accrued amount remains for recipient to withdraw
    ///
    /// # Panics
    /// - If stream is not `Active` or `Paused`
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
        let admin = get_admin(&env);
        admin.require_auth();

        let mut stream = load_stream(&env, stream_id)?;

        assert!(
            stream.status == StreamStatus::Active || stream.status == StreamStatus::Paused,
            "stream must be active or paused to cancel"
        );

        let accrued = Self::calculate_accrued(env.clone(), stream_id)?;
        let unstreamed = stream.deposit_amount - accrued;

        // CEI: update state before external token transfer to reduce reentrancy risk.
        // Assumption: the token contract does not reenter this contract.
        stream.status = StreamStatus::Cancelled;
        stream.cancelled_at = Some(env.ledger().timestamp());
        save_stream(&env, &stream);

        if unstreamed > 0 {
            push_token(&env, &stream.sender, unstreamed);
        }

        env.events().publish(
            (symbol_short!("cancelled"), stream_id),
            StreamEvent::StreamCancelled(stream_id),
        );
        Ok(())
    }

    /// Pause a payment stream as the contract admin.
    ///
    /// Administrative override to pause any stream, bypassing sender authorization.
    /// Identical behavior to `pause_stream` but requires admin authorization instead
    /// of sender authorization.
    ///
    /// # Parameters
    /// - `stream_id`: Unique identifier of the stream to pause
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
    /// - Publishes `Paused(stream_id)` event on success
    ///
    /// # Usage Notes
    /// - Admin can pause any stream regardless of sender
    /// - Accrual continues based on time (pause doesn't stop time)
    /// - Recipient cannot withdraw while paused
    pub fn pause_stream_as_admin(env: Env, stream_id: u64) -> Result<(), ContractError> {
        let admin = get_admin(&env);
        admin.require_auth();

        let mut stream = load_stream(&env, stream_id)?;

        assert!(
            stream.status == StreamStatus::Active,
            "stream is not active"
        );

        stream.status = StreamStatus::Paused;
        save_stream(&env, &stream);

        env.events().publish(
            (symbol_short!("paused"), stream_id),
            StreamEvent::Paused(stream_id),
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
        get_admin(&env).require_auth();
        let mut stream = load_stream(&env, stream_id)?;

        assert!(
            stream.status == StreamStatus::Paused,
            "stream is not paused"
        );

        stream.status = StreamStatus::Active;
        save_stream(&env, &stream);

        env.events().publish(
            (symbol_short!("resumed"), stream_id),
            StreamEvent::Resumed(stream_id),
        );
        Ok(())
    }
}

#[cfg(test)]
mod test;
