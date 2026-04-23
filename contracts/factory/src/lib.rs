#![no_std]

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
}

#[contracttype]
pub enum DataKey {
    Admin,
    StreamContract,
    MaxDepositCap,
    MinDuration,
    Allowlist(Address),
}

#[contract]
pub struct FluxoraFactory;

#[contractimpl]
impl FluxoraFactory {
    /// Initialize the factory with admin, stream contract, and policies.
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
        let admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(FactoryError::NotInitialized)?;
        admin.require_auth();

        env.storage().instance().set(&DataKey::Admin, &new_admin);
        Ok(())
    }

    /// Admin updates the stream contract address.
    pub fn set_stream_contract(env: Env, new_stream_contract: Address) -> Result<(), FactoryError> {
        let admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(FactoryError::NotInitialized)?;
        admin.require_auth();

        env.storage()
            .instance()
            .set(&DataKey::StreamContract, &new_stream_contract);
        Ok(())
    }

    /// Admin adds or removes a recipient from the allowlist.
    pub fn set_allowlist(env: Env, recipient: Address, allowed: bool) -> Result<(), FactoryError> {
        let admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(FactoryError::NotInitialized)?;
        admin.require_auth();

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
        let admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(FactoryError::NotInitialized)?;
        admin.require_auth();

        env.storage()
            .instance()
            .set(&DataKey::MaxDepositCap, &max_deposit);
        Ok(())
    }

    /// Admin updates the minimum stream duration.
    pub fn set_min_duration(env: Env, min_duration: u64) -> Result<(), FactoryError> {
        let admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(FactoryError::NotInitialized)?;
        admin.require_auth();

        env.storage()
            .instance()
            .set(&DataKey::MinDuration, &min_duration);
        Ok(())
    }

    /// Creates a new stream via the FluxoraStream contract after enforcing treasury policies.
    pub fn create_stream(
        env: Env,
        sender: Address,
        recipient: Address,
        deposit_amount: i128,
        rate_per_second: i128,
        start_time: u64,
        cliff_time: u64,
        end_time: u64,
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

        let min_duration: u64 = env
            .storage()
            .instance()
            .get(&DataKey::MinDuration)
            .ok_or(FactoryError::NotInitialized)?;
        let duration = if end_time > start_time {
            end_time - start_time
        } else {
            0
        };
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
            &None,
        );

        Ok(stream_id)
    }
}
