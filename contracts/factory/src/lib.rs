#![no_std]

extern crate alloc;

use soroban_sdk::{contract, contracterror, contractimpl, contracttype, Address, Env};

// ---------------------------------------------------------------------------
// Error codes
// ---------------------------------------------------------------------------

#[contracterror]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum FactoryError {
    NotInitialized = 1,
    AlreadyInitialized = 2,
    Unauthorized = 3,
}

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
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
#[derive(Clone, Debug, PartialEq)]
pub struct FactoryConfig {
    pub admin: Address,
    pub stream_contract: Address,
    pub max_deposit: i128,
    pub min_duration: u64,
}

#[contracttype]
pub enum DataKey {
    Admin,
    Policy,
    Allowlist(Address),
}

// ---------------------------------------------------------------------------
// Storage helpers
// ---------------------------------------------------------------------------

fn get_admin(env: &Env) -> Result<Address, FactoryError> {
    env.storage()
        .instance()
        .get(&DataKey::Admin)
        .ok_or(FactoryError::NotInitialized)
}

fn require_admin(env: &Env) -> Result<Address, FactoryError> {
    let admin = get_admin(env)?;
    admin.require_auth();
    Ok(admin)
}

fn get_policy(env: &Env) -> Result<FactoryPolicy, FactoryError> {
    env.storage()
        .instance()
        .get(&DataKey::Policy)
        .ok_or(FactoryError::NotInitialized)
}

fn save_policy(env: &Env, policy: &FactoryPolicy) {
    env.storage().instance().set(&DataKey::Policy, policy);
}

// ---------------------------------------------------------------------------
// Public helper (used by tests)
// ---------------------------------------------------------------------------

pub fn load_policy(env: &Env) -> Result<FactoryPolicy, FactoryError> {
    get_policy(env)
}

// ---------------------------------------------------------------------------
// Contract
// ---------------------------------------------------------------------------

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
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(FactoryError::AlreadyInitialized);
        }

        env.storage().instance().set(&DataKey::Admin, &admin);

        let policy = FactoryPolicy {
            stream_contract,
            max_deposit,
            min_duration,
            batch_cap_enforced: true,
            creation_paused: false,
            min_rate_per_second: None,
            max_rate_per_second: None,
        };
        save_policy(&env, &policy);

        Ok(())
    }

    // -- Views --

    pub fn get_factory_config(env: Env) -> Result<FactoryConfig, FactoryError> {
        let admin = get_admin(&env)?;
        let policy = get_policy(&env)?;
        Ok(FactoryConfig {
            admin,
            stream_contract: policy.stream_contract,
            max_deposit: policy.max_deposit,
            min_duration: policy.min_duration,
        })
    }

    pub fn is_allowlisted(env: Env, recipient: Address) -> bool {
        env.storage()
            .instance()
            .get::<_, bool>(&DataKey::Allowlist(recipient))
            .unwrap_or(false)
    }

    pub fn is_factory_paused(env: Env) -> Result<bool, FactoryError> {
        let policy = get_policy(&env)?;
        Ok(policy.creation_paused)
    }

    // -- Admin setters --

    pub fn set_admin(env: Env, new_admin: Address) -> Result<(), FactoryError> {
        require_admin(&env)?;
        env.storage().instance().set(&DataKey::Admin, &new_admin);
        Ok(())
    }

    pub fn set_stream_contract(env: Env, new_sc: Address) -> Result<(), FactoryError> {
        require_admin(&env)?;
        let mut policy = get_policy(&env)?;
        policy.stream_contract = new_sc;
        save_policy(&env, &policy);
        Ok(())
    }

    pub fn set_cap(env: Env, max_deposit: i128) -> Result<(), FactoryError> {
        require_admin(&env)?;
        let mut policy = get_policy(&env)?;
        policy.max_deposit = max_deposit;
        save_policy(&env, &policy);
        Ok(())
    }

    pub fn set_min_duration(env: Env, min_duration: u64) -> Result<(), FactoryError> {
        require_admin(&env)?;
        let mut policy = get_policy(&env)?;
        policy.min_duration = min_duration;
        save_policy(&env, &policy);
        Ok(())
    }

    pub fn set_allowlist(
        env: Env,
        recipient: Address,
        allowed: bool,
    ) -> Result<(), FactoryError> {
        require_admin(&env)?;
        env.storage()
            .instance()
            .set(&DataKey::Allowlist(recipient), &allowed);
        Ok(())
    }

    pub fn set_batch_cap_enforcement(
        env: Env,
        enforced: bool,
    ) -> Result<(), FactoryError> {
        require_admin(&env)?;
        let mut policy = get_policy(&env)?;
        policy.batch_cap_enforced = enforced;
        save_policy(&env, &policy);
        Ok(())
    }

    pub fn set_factory_paused(env: Env, paused: bool) -> Result<(), FactoryError> {
        require_admin(&env)?;
        let mut policy = get_policy(&env)?;
        policy.creation_paused = paused;
        save_policy(&env, &policy);
        Ok(())
    }

    pub fn set_rate_bounds(
        env: Env,
        min_rate: Option<i128>,
        max_rate: Option<i128>,
    ) -> Result<(), FactoryError> {
        require_admin(&env)?;
        let mut policy = get_policy(&env)?;
        policy.min_rate_per_second = min_rate;
        policy.max_rate_per_second = max_rate;
        save_policy(&env, &policy);
        Ok(())
    }
}
