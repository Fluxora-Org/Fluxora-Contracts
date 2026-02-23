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

    /// Create a new payment stream.
    ///
    /// Transfers `deposit_amount` of the stream token from `sender` to this
    /// contract and stores all stream parameters. Returns the new stream id.
    ///
    /// # Panics
    /// - If `deposit_amount` or `rate_per_second` is not positive.
    /// - If `sender` and `recipient` are the same address.
    /// - If `start_time >= end_time`.
    /// - If `cliff_time` is not in `[start_time, end_time]`.
    /// - If `deposit_amount < rate_per_second * (end_time - start_time)` (insufficient deposit).
    /// - If token transfer fails (e.g., insufficient balance or allowance).
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
    /// # Panics
    /// - If the stream is not in `Active` state.
    pub fn pause_stream(env: Env, stream_id: u64) {
        let mut stream = load_stream(&env, stream_id);

        // Corrected Auth Check
        Self::require_sender_or_admin(&env, &stream.sender);

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

    /// Resume a paused stream. Only the sender or admin may call this.
    /// # Panics
    /// - If the stream is `Active` (not paused).
    /// - If the stream is `Completed` (terminal state).
    /// - If the stream is `Cancelled` (terminal state).
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

    /// Cancel a stream and refund unstreamed funds to the sender.
    ///
    /// ## Behaviour
    /// 1. **Auth** — only the original sender or the contract admin can cancel.
    /// 2. **State check** — only `Active` or `Paused` streams can be cancelled.
    /// 3. **Accrual** — computes `accrued = min((now − start_time) × rate, deposit_amount)`.
    /// 4. **Refund** — transfers `deposit_amount − accrued` back to the sender immediately.
    /// 5. **Persistence** — the portion `accrued − withdrawn_amount` remains for the recipient.
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

    /// Withdraw accrued-but-not-yet-withdrawn tokens to the recipient.
    /// Returns the amount transferred.
    ///
    /// # Panics
    /// - If the stream is `Completed` (nothing left to withdraw).
    /// - If the stream is `Paused` (withdrawals not allowed while paused).
    /// - If there is nothing to withdraw (accrued == withdrawn).
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

        // // If the full deposit has been streamed and withdrawn, mark completed
        // let now = env.ledger().timestamp();
        // if stream.status == StreamStatus::Active
        //     && now >= stream.end_time
        //     && stream.withdrawn_amount == stream.deposit_amount
        // {
        //     stream.status = StreamStatus::Completed;
        // }

        if stream.withdrawn_amount >= stream.deposit_amount {
            stream.status = StreamStatus::Completed;
        }

        save_stream(&env, &stream);
        env.events()
            .publish((symbol_short!("withdrew"), stream_id), withdrawable);
        withdrawable
    }

    /// Calculate the total amount accrued to the recipient so far.
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

    /// Fetches the global configuration.
    pub fn get_config(env: Env) -> Config {
        get_config(&env)
    }

    /// Return the current state of the stream identified by `stream_id`.
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

    pub fn pause_stream_as_admin(env: Env, stream_id: u64) {
        get_admin(&env).require_auth();
        let mut stream = load_stream(&env, stream_id);
        assert!(
            stream.status == StreamStatus::Active,
            "stream is not active"
        );
        stream.status = StreamStatus::Paused;
        save_stream(&env, &stream);
        env.events()
            .publish((soroban_sdk::symbol_short!("paused"), stream_id), ());
    }

    pub fn resume_stream_as_admin(env: Env, stream_id: u64) {
        get_admin(&env).require_auth();
        let mut stream = load_stream(&env, stream_id);
        assert!(
            stream.status == StreamStatus::Paused,
            "stream is not paused"
        );
        stream.status = StreamStatus::Active;
        save_stream(&env, &stream);
        env.events()
            .publish((soroban_sdk::symbol_short!("resumed"), stream_id), ());
    }
}

#[cfg(test)]
mod test;
