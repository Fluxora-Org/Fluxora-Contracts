#![no_std]

use soroban_sdk::{contract, contracterror, contractimpl, contracttype, Address, Env};

/// Instance entries live for one network max-TTL window and are refreshed by
/// every mutating operation. A factory has no background keeper, so this is
/// the longest safe lease for configuration that may be idle.
const INSTANCE_TTL_LEDGERS: u32 = 17_280;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum FactoryError {
    NotInitialized = 1,
    AlreadyInitialized = 2,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FactoryConfig {
    pub admin: Address,
    pub stream_contract: Address,
    pub max_deposit: i128,
    pub min_duration: u64,
    pub batch_cap_enforced: bool,
    pub creation_paused: bool,
    pub min_rate_per_second: Option<i128>,
    pub max_rate_per_second: Option<i128>,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FactoryPolicy {
    pub stream_contract: Address,
    pub max_deposit: i128,
    pub min_duration: u64,
    pub batch_cap_enforced: bool,
    pub creation_paused: bool,
    pub min_rate_per_second: Option<i128>,
    pub max_rate_per_second: Option<i128>,
}

#[contracttype]
#[derive(Clone)]
enum DataKey {
    Initialized,
    Admin,
    StreamContract,
    MaxDeposit,
    MinDuration,
    BatchCapEnforced,
    CreationPaused,
    MinRatePerSecond,
    MaxRatePerSecond,
    Allowlist(Address),
}

#[contract]
pub struct FluxoraFactory;

#[contractimpl]
impl FluxoraFactory {
    pub fn init(
        env: Env,
        admin: Address,
        stream_contract: Address,
        max_deposit: i128,
        min_duration: u64,
    ) -> Result<(), FactoryError> {
        if initialized(&env) {
            return Err(FactoryError::AlreadyInitialized);
        }
        let storage = env.storage().instance();
        storage.set(&DataKey::Initialized, &true);
        storage.set(&DataKey::Admin, &admin);
        storage.set(&DataKey::StreamContract, &stream_contract);
        storage.set(&DataKey::MaxDeposit, &max_deposit);
        storage.set(&DataKey::MinDuration, &min_duration);
        storage.set(&DataKey::BatchCapEnforced, &true);
        storage.set(&DataKey::CreationPaused, &false);
        bump(&env);
        Ok(())
    }

    pub fn get_factory_config(env: Env) -> Result<FactoryConfig, FactoryError> {
        config(&env)
    }

    pub fn set_admin(env: Env, new_admin: Address) -> Result<(), FactoryError> {
        let old = admin(&env)?;
        old.require_auth();
        env.storage().instance().set(&DataKey::Admin, &new_admin);
        bump(&env);
        Ok(())
    }

    pub fn set_stream_contract(env: Env, value: Address) -> Result<(), FactoryError> {
        require_admin(&env)?;
        env.storage()
            .instance()
            .set(&DataKey::StreamContract, &value);
        bump(&env);
        Ok(())
    }

    pub fn set_cap(env: Env, value: i128) -> Result<(), FactoryError> {
        require_admin(&env)?;
        env.storage().instance().set(&DataKey::MaxDeposit, &value);
        bump(&env);
        Ok(())
    }

    pub fn set_min_duration(env: Env, value: u64) -> Result<(), FactoryError> {
        require_admin(&env)?;
        env.storage().instance().set(&DataKey::MinDuration, &value);
        bump(&env);
        Ok(())
    }

    pub fn set_allowlist(env: Env, address: Address, allowed: bool) -> Result<(), FactoryError> {
        require_admin(&env)?;
        env.storage()
            .instance()
            .set(&DataKey::Allowlist(address), &allowed);
        bump(&env);
        Ok(())
    }

    pub fn is_allowlisted(env: Env, address: Address) -> bool {
        env.storage()
            .instance()
            .get(&DataKey::Allowlist(address))
            .unwrap_or(false)
    }

    pub fn set_batch_cap_enforcement(env: Env, value: bool) -> Result<(), FactoryError> {
        require_admin(&env)?;
        env.storage()
            .instance()
            .set(&DataKey::BatchCapEnforced, &value);
        bump(&env);
        Ok(())
    }

    pub fn set_factory_paused(env: Env, value: bool) -> Result<(), FactoryError> {
        require_admin(&env)?;
        env.storage()
            .instance()
            .set(&DataKey::CreationPaused, &value);
        bump(&env);
        Ok(())
    }

    pub fn is_factory_paused(env: Env) -> Result<bool, FactoryError> {
        config(&env).map(|value| value.creation_paused)
    }

    pub fn set_rate_bounds(
        env: Env,
        min: Option<i128>,
        max: Option<i128>,
    ) -> Result<(), FactoryError> {
        require_admin(&env)?;
        env.storage()
            .instance()
            .set(&DataKey::MinRatePerSecond, &min);
        env.storage()
            .instance()
            .set(&DataKey::MaxRatePerSecond, &max);
        bump(&env);
        Ok(())
    }
}

pub fn load_policy(env: &Env) -> Result<FactoryPolicy, FactoryError> {
    let value = config(env)?;
    Ok(FactoryPolicy {
        stream_contract: value.stream_contract,
        max_deposit: value.max_deposit,
        min_duration: value.min_duration,
        batch_cap_enforced: value.batch_cap_enforced,
        creation_paused: value.creation_paused,
        min_rate_per_second: value.min_rate_per_second,
        max_rate_per_second: value.max_rate_per_second,
    })
}

fn initialized(env: &Env) -> bool {
    env.storage()
        .instance()
        .get(&DataKey::Initialized)
        .unwrap_or(false)
}

fn admin(env: &Env) -> Result<Address, FactoryError> {
    if !initialized(env) {
        return Err(FactoryError::NotInitialized);
    }
    env.storage()
        .instance()
        .get(&DataKey::Admin)
        .ok_or(FactoryError::NotInitialized)
}

fn require_admin(env: &Env) -> Result<Address, FactoryError> {
    let value = admin(env)?;
    value.require_auth();
    Ok(value)
}

fn config(env: &Env) -> Result<FactoryConfig, FactoryError> {
    Ok(FactoryConfig {
        admin: admin(env)?,
        stream_contract: env
            .storage()
            .instance()
            .get(&DataKey::StreamContract)
            .ok_or(FactoryError::NotInitialized)?,
        max_deposit: env
            .storage()
            .instance()
            .get(&DataKey::MaxDeposit)
            .ok_or(FactoryError::NotInitialized)?,
        min_duration: env
            .storage()
            .instance()
            .get(&DataKey::MinDuration)
            .ok_or(FactoryError::NotInitialized)?,
        batch_cap_enforced: env
            .storage()
            .instance()
            .get(&DataKey::BatchCapEnforced)
            .unwrap_or(true),
        creation_paused: env
            .storage()
            .instance()
            .get(&DataKey::CreationPaused)
            .unwrap_or(false),
        min_rate_per_second: env.storage().instance().get(&DataKey::MinRatePerSecond),
        max_rate_per_second: env.storage().instance().get(&DataKey::MaxRatePerSecond),
    })
}

fn bump(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(INSTANCE_TTL_LEDGERS, INSTANCE_TTL_LEDGERS);
}
