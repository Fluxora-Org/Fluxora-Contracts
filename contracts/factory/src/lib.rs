#![no_std]
#![allow(clippy::too_many_arguments)]

use fluxora_stream::FluxoraStreamClient;
use soroban_sdk::{contract, contracterror, contractimpl, contracttype, Address, Env};

/// Maximum accepted value for the factory `min_duration` policy, in seconds.
///
/// The ceiling is intentionally generous (100 years, using 365-day years) so
/// normal treasury vesting schedules remain valid while malformed policies
/// cannot silently make factory-routed stream creation impractical forever.
pub const MAX_MIN_DURATION_SECONDS: u64 = 100 * 365 * 24 * 60 * 60;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum FactoryError {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    Unauthorized = 3,
    RecipientNotAllowlisted = 4,
    DepositExceedsCap = 5,
    DurationTooShort = 6,
    /// The requested stream must end strictly after it starts.
    InvalidTimeRange = 7,
    /// The requested cliff must be within the inclusive start/end window.
    InvalidCliff = 8,
    /// The factory cap must be in the accepted range `1..=i128::MAX`.
    InvalidCap = 9,
    /// The minimum duration must be in the accepted range
    /// `0..=MAX_MIN_DURATION_SECONDS` seconds.
    InvalidMinDuration = 10,
}

#[contracttype]
pub enum DataKey {
    Admin,
    StreamContract,
    MaxDepositCap,
    MinDuration,
    Allowlist(Address),
}

/// Load and authorize the current factory admin.
///
/// This is the single authorization chokepoint for admin-only factory setters.
/// It preserves the existing `NotInitialized` behavior before attempting auth.
fn require_admin(env: &Env) -> Result<Address, FactoryError> {
    let admin: Address = env
        .storage()
        .instance()
        .get(&DataKey::Admin)
        .ok_or(FactoryError::NotInitialized)?;
    admin.require_auth();
    Ok(admin)
}

/// Validate a factory deposit cap before storing it.
///
/// Accepted range: `1..=i128::MAX`. A non-positive cap would make every
/// positive stream deposit exceed the cap, effectively bricking factory-routed
/// stream creation.
fn validate_cap(max_deposit: i128) -> Result<(), FactoryError> {
    if max_deposit <= 0 {
        return Err(FactoryError::InvalidCap);
    }

    Ok(())
}

/// Validate a factory minimum-duration policy before storing it.
///
/// Accepted range: `0..=MAX_MIN_DURATION_SECONDS` seconds. A value of `0`
/// disables any additional factory-level minimum duration while `create_stream`
/// still enforces `start_time < end_time`.
fn validate_min_duration(min_duration: u64) -> Result<(), FactoryError> {
    if min_duration > MAX_MIN_DURATION_SECONDS {
        return Err(FactoryError::InvalidMinDuration);
    }

    Ok(())
}

/// Read-only snapshot of the factory policy stored in instance storage.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FactoryConfig {
    pub admin: Address,
    pub stream_contract: Address,
    pub max_deposit: i128,
    pub min_duration: u64,
}

#[contract]
pub struct FluxoraFactory;

#[contractimpl]
#[allow(clippy::too_many_arguments)]
impl FluxoraFactory {
    /// Initialize the factory with admin, stream contract, and policies.
    ///
    /// Accepted policy ranges:
    /// - `max_deposit`: `1..=i128::MAX` (`FactoryError::InvalidCap` otherwise).
    /// - `min_duration`: `0..=MAX_MIN_DURATION_SECONDS` seconds
    ///   (`FactoryError::InvalidMinDuration` otherwise).
    pub fn init(
        env: Env,
        admin: Address,
        stream_contract: Address,
        max_deposit: i128,
        min_duration: u64,
    ) -> Result<(), FactoryError> {
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(FactoryError::AlreadyInitialized);
        }

        validate_cap(max_deposit)?;
        validate_min_duration(min_duration)?;

        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage()
            .instance()
            .set(&DataKey::StreamContract, &stream_contract);
        env.storage()
            .instance()
            .set(&DataKey::MaxDepositCap, &max_deposit);
        env.storage()
            .instance()
            .set(&DataKey::MinDuration, &min_duration);

        Ok(())
    }

    /// Admin updates the factory admin.
    pub fn set_admin(env: Env, new_admin: Address) -> Result<(), FactoryError> {
        require_admin(&env)?;

        env.storage().instance().set(&DataKey::Admin, &new_admin);
        Ok(())
    }

    /// Admin updates the stream contract address.
    pub fn set_stream_contract(env: Env, new_stream_contract: Address) -> Result<(), FactoryError> {
        require_admin(&env)?;

        env.storage()
            .instance()
            .set(&DataKey::StreamContract, &new_stream_contract);
        Ok(())
    }

    /// Admin adds or removes a recipient from the allowlist.
    pub fn set_allowlist(env: Env, recipient: Address, allowed: bool) -> Result<(), FactoryError> {
        require_admin(&env)?;

        let key = DataKey::Allowlist(recipient);
        if allowed {
            env.storage().persistent().set(&key, &true);
        } else {
            env.storage().persistent().remove(&key);
        }

        Ok(())
    }

    /// Admin updates the max deposit cap.
    ///
    /// Accepted range: `1..=i128::MAX`. Non-positive values return
    /// `FactoryError::InvalidCap` and leave the stored cap unchanged.
    pub fn set_cap(env: Env, max_deposit: i128) -> Result<(), FactoryError> {
        require_admin(&env)?;
        validate_cap(max_deposit)?;

        env.storage()
            .instance()
            .set(&DataKey::MaxDepositCap, &max_deposit);
        Ok(())
    }

    /// Admin updates the minimum stream duration.
    ///
    /// Accepted range: `0..=MAX_MIN_DURATION_SECONDS` seconds. A value of `0`
    /// disables any additional factory-level minimum duration; values above the
    /// ceiling return `FactoryError::InvalidMinDuration` and leave the stored
    /// policy unchanged.
    pub fn set_min_duration(env: Env, min_duration: u64) -> Result<(), FactoryError> {
        require_admin(&env)?;
        validate_min_duration(min_duration)?;

        env.storage()
            .instance()
            .set(&DataKey::MinDuration, &min_duration);
        Ok(())
    }

    /// Return the current factory policy configuration.
    pub fn get_factory_config(env: Env) -> Result<FactoryConfig, FactoryError> {
        Ok(FactoryConfig {
            admin: env
                .storage()
                .instance()
                .get(&DataKey::Admin)
                .ok_or(FactoryError::NotInitialized)?,
            stream_contract: env
                .storage()
                .instance()
                .get(&DataKey::StreamContract)
                .ok_or(FactoryError::NotInitialized)?,
            max_deposit: env
                .storage()
                .instance()
                .get(&DataKey::MaxDepositCap)
                .ok_or(FactoryError::NotInitialized)?,
            min_duration: env
                .storage()
                .instance()
                .get(&DataKey::MinDuration)
                .ok_or(FactoryError::NotInitialized)?,
        })
    }

    /// Return whether `recipient` is currently allowlisted for factory-created streams.
    pub fn is_allowlisted(env: Env, recipient: Address) -> bool {
        env.storage()
            .persistent()
            .get(&DataKey::Allowlist(recipient))
            .unwrap_or(false)
    }

    /// Creates a new stream via the FluxoraStream contract after enforcing treasury policies.
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
    ) -> Result<u64, FactoryError> {
        // Enforce policies
        let is_allowed: bool = env
            .storage()
            .persistent()
            .get(&DataKey::Allowlist(recipient.clone()))
            .unwrap_or(false);
        if !is_allowed {
            return Err(FactoryError::RecipientNotAllowlisted);
        }

        let max_deposit: i128 = env
            .storage()
            .instance()
            .get(&DataKey::MaxDepositCap)
            .ok_or(FactoryError::NotInitialized)?;
        if deposit_amount > max_deposit {
            return Err(FactoryError::DepositExceedsCap);
        }

        // Mirror FluxoraStream time invariants before the cross-contract call so
        // invalid schedules return typed factory errors instead of downstream panics.
        if start_time >= end_time {
            return Err(FactoryError::InvalidTimeRange);
        }
        if cliff_time < start_time || cliff_time > end_time {
            return Err(FactoryError::InvalidCliff);
        }

        let min_duration: u64 = env
            .storage()
            .instance()
            .get(&DataKey::MinDuration)
            .ok_or(FactoryError::NotInitialized)?;
        let duration = end_time - start_time;
        if duration < min_duration {
            return Err(FactoryError::DurationTooShort);
        }

        // Must authenticate the sender because the factory calls FluxoraStream with this sender.
        // The sender needs to authorize both this wrapper invocation and the cross-contract invocation.
        sender.require_auth();

        let stream_contract: Address = env
            .storage()
            .instance()
            .get(&DataKey::StreamContract)
            .ok_or(FactoryError::NotInitialized)?;

        let stream_client = FluxoraStreamClient::new(&env, &stream_contract);

        // We wrap the `try_create_stream` to gracefully handle underlying failures if needed,
        // but for now, `.create_stream()` automatically panics with the underlying contract error
        // if it fails, which is standard Soroban cross-contract call behavior.
        let stream_id = stream_client.create_stream(
            &sender,
            &recipient,
            &deposit_amount,
            &rate_per_second,
            &start_time,
            &cliff_time,
            &end_time,
            &withdraw_dust_threshold,
            &None,
            &fluxora_stream::StreamKind::Linear,
        );

        Ok(stream_id)
    }
}
