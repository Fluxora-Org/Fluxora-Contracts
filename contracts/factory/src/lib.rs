#![no_std]
#![allow(clippy::too_many_arguments)]

use fluxora_stream::FluxoraStreamClient;
use soroban_sdk::{contract, contracterror, contractimpl, contracttype, Address, Env};

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
    /// The supplied `stream_contract` address did not respond to the
    /// `FluxoraStream::version()` smoke check (e.g. it is not a deployed
    /// contract, or does not implement the `FluxoraStream` interface).
    ///
    /// Returned by `init` and `set_stream_contract` instead of letting an
    /// invalid address be persisted and later host-trap inside `create_stream`.
    InvalidStreamContract = 9,
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

/// Smoke-test a candidate stream contract for the `FluxoraStream` interface.
///
/// This helper is called from `init` and `set_stream_contract` before the
/// candidate address is persisted as the factory's `StreamContract`. It
/// invokes the read-only, storage-free `version()` entrypoint via
/// `FluxoraStreamClient::try_version`, which uses `Env::try_invoke_contract`
/// internally so a missing contract, an EOA address, or a contract that does
/// not expose `version()` surfaces as a typed `FactoryError` instead of a
/// host trap.
///
/// `version()` is intentionally cheap to check: it performs no storage reads
/// and works even on a `FluxoraStream` contract that has not yet been
/// initialized, so it is safe to call during the factory's own bootstrap.
fn validate_stream_contract(env: &Env, stream_contract: &Address) -> Result<(), FactoryError> {
    let client = FluxoraStreamClient::new(env, stream_contract);
    match client.try_version() {
        Ok(Ok(_)) => Ok(()),
        _ => Err(FactoryError::InvalidStreamContract),
    }
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
    /// # Authorization
    /// The declared `admin` must authorize this call via `admin.require_auth()`.
    /// This matches every other admin-only entrypoint (`set_admin`,
    /// `set_stream_contract`, `set_allowlist`, `set_cap`, `set_min_duration`,
    /// all of which go through `require_admin`) and prevents an unrelated
    /// caller from front-running bootstrap by seeding the factory with an
    /// admin address they do not control.
    ///
    /// # Validation
    /// `stream_contract` must pass the `FluxoraStream` smoke check (see
    /// [`validate_stream_contract`]) before it is persisted. This converts a
    /// misconfigured stream address from a deferred host trap inside
    /// `create_stream` into an immediate `FactoryError::InvalidStreamContract`
    /// at setup time.
    ///
    /// # Errors
    /// - `FactoryError::AlreadyInitialized` if `init` has already succeeded.
    /// - `FactoryError::InvalidStreamContract` if `stream_contract` does not
    ///   respond to `FluxoraStream::version()`.
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

        admin.require_auth();
        validate_stream_contract(&env, &stream_contract)?;

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
    ///
    /// # Validation
    /// `new_stream_contract` must pass the same `FluxoraStream` smoke check
    /// applied in `init` (see [`validate_stream_contract`]), so a later swap
    /// cannot silently install a non-`FluxoraStream` address. On failure the
    /// previously configured `stream_contract` is left untouched.
    pub fn set_stream_contract(env: Env, new_stream_contract: Address) -> Result<(), FactoryError> {
        require_admin(&env)?;
        validate_stream_contract(&env, &new_stream_contract)?;

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
    pub fn set_cap(env: Env, max_deposit: i128) -> Result<(), FactoryError> {
        require_admin(&env)?;

        env.storage()
            .instance()
            .set(&DataKey::MaxDepositCap, &max_deposit);
        Ok(())
    }

    /// Admin updates the minimum stream duration.
    pub fn set_min_duration(env: Env, min_duration: u64) -> Result<(), FactoryError> {
        require_admin(&env)?;

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
