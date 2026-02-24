#![no_std]

mod accrual;

use soroban_sdk::{contract, contractimpl, contracttype, symbol_short, token, Address, Env};

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

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StreamEvent {
    Paused(u64),
    Resumed(u64),
    Cancelled(u64),
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
}

/// Namespace for all contract storage keys.
#[contracttype]
pub enum DataKey {
    Config,       // Instance storage for global settings (admin/token).
    NextStreamId, // Instance storage for the auto-incrementing ID counter.
    Stream(u64),  // Persistent storage for individual stream data (O(1) lookup).
}

// ---------------------------------------------------------------------------
// Storage helpers
// ---------------------------------------------------------------------------

fn get_config(env: &Env) -> Config {
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

fn get_stream_count(env: &Env) -> u64 {
    env.storage()
        .instance()
        .get(&DataKey::NextStreamId)
        .unwrap_or(0u64)
}

fn set_stream_count(env: &Env, count: u64) {
    env.storage().instance().set(&DataKey::NextStreamId, &count);
}

fn load_stream(env: &Env, stream_id: u64) -> Stream {
    env.storage()
        .persistent()
        .get(&DataKey::Stream(stream_id))
        .expect("stream not found")
}

fn save_stream(env: &Env, stream: &Stream) {
    let key = DataKey::Stream(stream.stream_id);
    env.storage().persistent().set(&key, stream);

    // Requirement from Issue #1: extend TTL on stream save to ensure persistence
    env.storage().persistent().extend_ttl(&key, 17280, 120960);
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

        // Ensure instance storage (Config/ID) doesn't expire quickly
        env.storage().instance().extend_ttl(17280, 120960);
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
    /// - `deposit_amount`: Total tokens to deposit (must be > 0)
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
    /// - `cliff_time` in `[start_time, end_time]` (cliff within stream duration)
    /// - `deposit_amount >= rate_per_second × (end_time - start_time)` (sufficient deposit)
    ///
    /// # Panics
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

        // Transfer tokens from sender to this contract (#36)
        // If transfer fails (insufficient balance/allowance), this will panic
        // and no state will be persisted (atomic transaction)
        let token_client = token::Client::new(&env, &get_token(&env));
        token_client.transfer(&sender, &env.current_contract_address(), &deposit_amount);

        // Only allocate stream id and persist state AFTER successful transfer
        let stream_id = get_stream_count(&env);
        set_stream_count(&env, stream_id + 1);

        let stream = Stream {
            stream_id,
            sender,
            recipient,
            deposit_amount,
            rate_per_second,
            start_time,
            cliff_time,
            end_time,
            withdrawn_amount: 0,
            status: StreamStatus::Active,
        };

        save_stream(&env, &stream);

        env.events()
            .publish((symbol_short!("created"), stream_id), deposit_amount);

        stream_id
    }

    /// Pause an active stream. Only the sender or admin may call this.
    ///
    /// ## Errors / Panics
    /// - Panics with `"stream is already paused"` if the stream is already in
    ///   `Paused` state. This is a clear, distinct error to aid frontend/backend
    ///   consumers in diagnosing double-pause scenarios.
    /// - Panics with `"stream must be active to pause"` if the stream is in any
    ///   other non-`Active` state (`Completed` or `Cancelled`).
    ///
    /// ## Consistency with `resume_stream`
    /// `resume_stream` mirrors this behaviour: it panics with
    /// `"stream is not paused"` when called on a non-`Paused` stream.
    pub fn pause_stream(env: Env, stream_id: u64) {
        let mut stream = load_stream(&env, stream_id);

        Self::require_sender_or_admin(&env, &stream.sender);

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
    pub fn resume_stream(env: Env, stream_id: u64) {
        let mut stream = load_stream(&env, stream_id);
        Self::require_sender_or_admin(&env, &stream.sender);

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
    /// # Examples
    /// - Cancel at 30% completion → sender gets 70% refund, recipient can withdraw 30%
    /// - Cancel at 100% completion → sender gets 0% refund, recipient can withdraw 100%
    /// - Cancel before cliff → sender gets 100% refund (no accrual before cliff)
    pub fn cancel_stream(env: Env, stream_id: u64) {
        let mut stream = load_stream(&env, stream_id);
        Self::require_sender_or_admin(&env, &stream.sender);

        assert!(
            stream.status == StreamStatus::Active || stream.status == StreamStatus::Paused,
            "stream must be active or paused to cancel"
        );

        let accrued = Self::calculate_accrued(env.clone(), stream_id);
        let unstreamed = stream.deposit_amount - accrued;

        if unstreamed > 0 {
            let token_client = token::Client::new(&env, &get_token(&env));
            token_client.transfer(&env.current_contract_address(), &stream.sender, &unstreamed);
        }

        stream.status = StreamStatus::Cancelled;
        save_stream(&env, &stream);

        env.events().publish(
            (symbol_short!("cancelled"), stream_id),
            StreamEvent::Cancelled(stream_id),
        );
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
    /// - `i128`: The amount of tokens transferred to the recipient
    ///
    /// # Authorization
    /// - Requires authorization from the stream's recipient (only recipient can withdraw)
    /// - This prevents anyone from withdrawing on behalf of the recipient
    ///
    /// # Panics
    /// - If the stream is `Completed` (all tokens already withdrawn)
    /// - If the stream is `Paused` (withdrawals not allowed while paused)
    /// - If there is nothing to withdraw (`accrued == withdrawn_amount`)
    /// - If the stream does not exist (`stream_id` is invalid)
    /// - If caller is not authorized (not the recipient)
    /// - If token transfer fails (insufficient contract balance, should not happen)
    ///
    /// # State Changes
    /// - Updates `withdrawn_amount` by the amount transferred
    /// - Sets status to `Completed` if all deposited tokens are withdrawn
    /// - Extends stream storage TTL to prevent expiration
    ///
    /// # Events
    /// - Publishes `withdrew(stream_id, amount)` event on success
    ///
    /// # Usage Notes
    /// - Can be called multiple times to withdraw incrementally
    /// - Accrual is time-based: `min((now - start_time) × rate, deposit_amount)`
    /// - Before cliff time, accrued amount is 0 (nothing to withdraw)
    /// - After end_time, accrued amount is capped at deposit_amount
    /// - Works on `Active` and `Cancelled` streams, not on `Paused` or `Completed`
    /// - For cancelled streams, only the accrued amount (not refunded) can be withdrawn
    ///
    /// # Examples
    /// - Stream: 1000 tokens over 1000 seconds (1 token/sec)
    /// - At t=300: withdraw() returns 300 tokens
    /// - At t=800: withdraw() returns 500 tokens (800 - 300 already withdrawn)
    /// - At t=1000: withdraw() returns 200 tokens, status → Completed
    pub fn withdraw(env: Env, stream_id: u64) -> i128 {
        let mut stream = load_stream(&env, stream_id);

        // Enforce recipient-only authorization: only the stream's recipient can withdraw
        // This is equivalent to checking env.invoker() == stream.recipient
        // require_auth() ensures only the recipient can authorize this call,
        // preventing anyone from withdrawing on behalf of the recipient
        stream.recipient.require_auth();

        assert!(
            stream.status != StreamStatus::Completed,
            "stream already completed"
        );

        assert!(
            stream.status != StreamStatus::Paused,
            "cannot withdraw from paused stream"
        );

        let accrued = Self::calculate_accrued(env.clone(), stream_id);
        let withdrawable = accrued - stream.withdrawn_amount;
        assert!(withdrawable > 0, "nothing to withdraw");

        let token_client = token::Client::new(&env, &get_token(&env));
        token_client.transfer(
            &env.current_contract_address(),
            &stream.recipient,
            &withdrawable,
        );

        stream.withdrawn_amount += withdrawable;

        // If the full deposit has been streamed and withdrawn, mark completed.
        if stream.withdrawn_amount == stream.deposit_amount {
            stream.status = StreamStatus::Completed;
        }

        save_stream(&env, &stream);
        env.events()
            .publish((symbol_short!("withdrew"), stream_id), withdrawable);
        withdrawable
    }

    /// Calculate the total amount accrued to the recipient at the current time.
    ///
    /// Computes how many tokens have been earned by the recipient based on time elapsed
    /// since the stream started. This is a read-only view function that does not modify
    /// state or require authorization.
    ///
    /// # Parameters
    /// - `stream_id`: Unique identifier of the stream to query
    ///
    /// # Returns
    /// - `i128`: Total accrued amount in tokens (not the withdrawable amount)
    ///
    /// # Calculation
    /// - Before `cliff_time`: returns 0 (no accrual before cliff)
    /// - After `cliff_time`: `min((now - start_time) × rate_per_second, deposit_amount)`
    /// - After `end_time`: capped at `deposit_amount` (no accrual beyond end)
    ///
    /// # Panics
    /// - If the stream does not exist (`stream_id` is invalid)
    ///
    /// # Usage Notes
    /// - This is a view function (read-only, no state changes)
    /// - No authorization required (public information)
    /// - Returns total accrued, not withdrawable amount
    /// - To get withdrawable amount: `calculate_accrued() - stream.withdrawn_amount`
    /// - Accrual is time-based and unaffected by stream status (Active, Paused, etc.)
    /// - Useful for UIs to show real-time accrual without transactions
    ///
    /// # Examples
    /// - Stream: 1000 tokens, 0-1000s, rate 1 token/sec, cliff at 500s
    /// - At t=300: returns 0 (before cliff)
    /// - At t=500: returns 500 (at cliff, accrual from start_time)
    /// - At t=800: returns 800
    /// - At t=1500: returns 1000 (capped at deposit_amount)
    pub fn calculate_accrued(env: Env, stream_id: u64) -> i128 {
        let stream = load_stream(&env, stream_id);
        let now = env.ledger().timestamp();

        accrual::calculate_accrued_amount(
            stream.start_time,
            stream.cliff_time,
            stream.end_time,
            stream.rate_per_second,
            stream.deposit_amount,
            now,
        )
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
    /// - Config is set once during `init()` and cannot be changed
    /// - Useful for integrators to verify token and admin addresses
    pub fn get_config(env: Env) -> Config {
        get_config(&env)
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
        load_stream(&env, stream_id)
    }

    /// Internal helper to check authorization for sender or admin.
    fn require_sender_or_admin(_env: &Env, sender: &Address) {
        // Only the sender can manage their own stream via these paths.
        // Admin overrides are handled by the 'as_admin' specific functions.
        sender.require_auth();
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
    pub fn cancel_stream_as_admin(env: Env, stream_id: u64) {
        let admin = get_admin(&env);
        admin.require_auth();

        let mut stream = load_stream(&env, stream_id);

        assert!(
            stream.status == StreamStatus::Active || stream.status == StreamStatus::Paused,
            "stream must be active or paused to cancel"
        );

        let accrued = Self::calculate_accrued(env.clone(), stream_id);
        let unstreamed = stream.deposit_amount - accrued;

        if unstreamed > 0 {
            let token_client = token::Client::new(&env, &get_token(&env));
            token_client.transfer(&env.current_contract_address(), &stream.sender, &unstreamed);
        }

        stream.status = StreamStatus::Cancelled;
        save_stream(&env, &stream);

        env.events().publish(
            (symbol_short!("cancelled"), stream_id),
            StreamEvent::Cancelled(stream_id),
        );
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
    pub fn pause_stream_as_admin(env: Env, stream_id: u64) {
        let admin = get_admin(&env);
        admin.require_auth();

        let mut stream = load_stream(&env, stream_id);

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
    pub fn resume_stream_as_admin(env: Env, stream_id: u64) {
        get_admin(&env).require_auth();
        let mut stream = load_stream(&env, stream_id);

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
    }
}

#[cfg(test)]
mod test;
