#![no_std]

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, symbol_short, token, Address, Env,
};

// ---------------------------------------------------------------------------
// Error types
// ---------------------------------------------------------------------------

#[contracterror]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    Unauthorized = 3,
    StreamNotFound = 4,
    InvalidState = 5,
    InvalidParams = 6,
    InsufficientBalance = 7,
    NothingToWithdraw = 8,
    ArithmeticOverflow = 9,
}

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

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

#[contracttype]
pub enum DataKey {
    Config,
    NextStreamId,
    Stream(u64),
}

// ---------------------------------------------------------------------------
// Storage helpers
// ---------------------------------------------------------------------------

fn get_config(env: &Env) -> Result<Config, Error> {
    env.storage()
        .instance()
        .get(&DataKey::Config)
        .ok_or(Error::NotInitialized)
}

fn get_token(env: &Env) -> Result<Address, Error> {
    get_config(env).map(|c| c.token)
}

fn get_admin(env: &Env) -> Result<Address, Error> {
    get_config(env).map(|c| c.admin)
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

fn load_stream(env: &Env, stream_id: u64) -> Result<Stream, Error> {
    env.storage()
        .persistent()
        .get(&DataKey::Stream(stream_id))
        .ok_or(Error::StreamNotFound)
}

fn save_stream(env: &Env, stream: &Stream) {
    let key = DataKey::Stream(stream.stream_id);
    env.storage().persistent().set(&key, stream);
    env.storage().persistent().extend_ttl(&key, 17280, 120960);
}

// ---------------------------------------------------------------------------
// Contract Implementation
// ---------------------------------------------------------------------------

#[contract]
pub struct FluxoraStream;

#[contractimpl]
impl FluxoraStream {
    pub fn init(env: Env, token: Address, admin: Address) -> Result<(), Error> {
        if env.storage().instance().has(&DataKey::Config) {
            return Err(Error::AlreadyInitialized);
        }
        let config = Config { token, admin };
        env.storage().instance().set(&DataKey::Config, &config);
        env.storage().instance().set(&DataKey::NextStreamId, &0u64);
        env.storage().instance().extend_ttl(17280, 120960);
        Ok(())
    }

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
    ) -> Result<u64, Error> {
        sender.require_auth();

        if deposit_amount <= 0 {
            return Err(Error::InvalidParams);
        }
        if rate_per_second <= 0 {
            return Err(Error::InvalidParams);
        }
        if sender == recipient {
            return Err(Error::InvalidParams);
        }
        if start_time >= end_time {
            return Err(Error::InvalidParams);
        }
        if cliff_time < start_time || cliff_time > end_time {
            return Err(Error::InvalidParams);
        }

        let duration = (end_time - start_time) as i128;
        let total_streamable = rate_per_second
            .checked_mul(duration)
            .ok_or(Error::ArithmeticOverflow)?;
        if deposit_amount < total_streamable {
            return Err(Error::InvalidParams);
        }

        let token = get_token(&env)?;
        let token_client = token::Client::new(&env, &token);

        let sender_balance = token_client.balance(&sender);
        if sender_balance < deposit_amount {
            return Err(Error::InsufficientBalance);
        }

        token_client.transfer(&sender, &env.current_contract_address(), &deposit_amount);

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

        Ok(stream_id)
    }

    pub fn pause_stream(env: Env, stream_id: u64) -> Result<(), Error> {
        let mut stream = load_stream(&env, stream_id)?;
        Self::require_sender_or_admin(&env, &stream.sender)?;

        if stream.status != StreamStatus::Active {
            return Err(Error::InvalidState);
        }

        stream.status = StreamStatus::Paused;
        save_stream(&env, &stream);

        env.events()
            .publish((symbol_short!("paused"), stream_id), ());

        Ok(())
    }

    pub fn resume_stream(env: Env, stream_id: u64) -> Result<(), Error> {
        let mut stream = load_stream(&env, stream_id)?;
        Self::require_sender_or_admin(&env, &stream.sender)?;

        if stream.status != StreamStatus::Paused {
            return Err(Error::InvalidState);
        }

        stream.status = StreamStatus::Active;
        save_stream(&env, &stream);

        env.events()
            .publish((symbol_short!("resumed"), stream_id), ());

        Ok(())
    }

    pub fn cancel_stream(env: Env, stream_id: u64) -> Result<(), Error> {
        let mut stream = load_stream(&env, stream_id)?;
        Self::require_sender_or_admin(&env, &stream.sender)?;

        if stream.status != StreamStatus::Active && stream.status != StreamStatus::Paused {
            return Err(Error::InvalidState);
        }

        let accrued = Self::calculate_accrued(env.clone(), stream_id)?;
        let unstreamed = stream.deposit_amount.saturating_sub(accrued);

        if unstreamed > 0 {
            let token = get_token(&env)?;
            let token_client = token::Client::new(&env, &token);
            token_client.transfer(&env.current_contract_address(), &stream.sender, &unstreamed);
        }

        stream.status = StreamStatus::Cancelled;
        save_stream(&env, &stream);

        env.events()
            .publish((symbol_short!("cancelled"), stream_id), unstreamed);

        Ok(())
    }

    pub fn withdraw(env: Env, stream_id: u64) -> Result<i128, Error> {
        let mut stream = load_stream(&env, stream_id)?;
        stream.recipient.require_auth();

        if stream.status == StreamStatus::Completed {
            return Err(Error::InvalidState);
        }
        if stream.status == StreamStatus::Paused {
            return Err(Error::InvalidState);
        }

        let accrued = Self::calculate_accrued(env.clone(), stream_id)?;
        let withdrawable = accrued.saturating_sub(stream.withdrawn_amount);

        if withdrawable <= 0 {
            return Err(Error::NothingToWithdraw);
        }

        let token = get_token(&env)?;
        let token_client = token::Client::new(&env, &token);
        token_client.transfer(
            &env.current_contract_address(),
            &stream.recipient,
            &withdrawable,
        );

        stream.withdrawn_amount = stream.withdrawn_amount.saturating_add(withdrawable);

        if stream.status == StreamStatus::Active
            && env.ledger().timestamp() >= stream.end_time
            && stream.withdrawn_amount == stream.deposit_amount
        {
            stream.status = StreamStatus::Completed;
        }

        save_stream(&env, &stream);
        env.events()
            .publish((symbol_short!("withdrew"), stream_id), withdrawable);
        Ok(withdrawable)
    }

    pub fn calculate_accrued(env: Env, stream_id: u64) -> Result<i128, Error> {
        let stream = load_stream(&env, stream_id)?;
        let now = env.ledger().timestamp();

        if now < stream.cliff_time {
            return Ok(0);
        }

        let elapsed = (now.min(stream.end_time)).saturating_sub(stream.start_time) as i128;
        let accrued = elapsed.saturating_mul(stream.rate_per_second);

        Ok(accrued.min(stream.deposit_amount))
    }

    pub fn get_config(env: Env) -> Result<Config, Error> {
        get_config(&env)
    }

    pub fn get_stream_state(env: Env, stream_id: u64) -> Result<Stream, Error> {
        load_stream(&env, stream_id)
    }

    fn require_sender_or_admin(env: &Env, sender: &Address) -> Result<(), Error> {
        let admin = get_admin(env)?;

        if sender != &admin {
            sender.require_auth();
        } else {
            admin.require_auth();
        }
        Ok(())
    }
}

#[contractimpl]
impl FluxoraStream {
    pub fn cancel_stream_as_admin(env: Env, stream_id: u64) -> Result<(), Error> {
        get_admin(&env)?.require_auth();
        Self::cancel_stream(env, stream_id)
    }
}

#[cfg(test)]
mod test;
