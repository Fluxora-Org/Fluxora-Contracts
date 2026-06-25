#![no_std]
#![allow(clippy::too_many_arguments)]

use fluxora_stream::FluxoraStreamClient;
use soroban_sdk::{contract, contracterror, contractimpl, contracttype, Address, Env, Vec};

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
    /// The requested memo exceeds the allowed max length.
    InvalidMemo = 9,
}

#[contracttype]
pub enum DataKey {
    Admin,
    StreamContract,
    MaxDepositCap,
    MinDuration,
    BatchCapEnforced,
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

/// Read-only snapshot of the factory policy stored in instance storage.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FactoryConfig {
    pub admin: Address,
    pub stream_contract: Address,
    pub max_deposit: i128,
    pub min_duration: u64,
    pub batch_cap_enforced: bool,
}

#[contract]
pub struct FluxoraFactory;

#[contractimpl]
#[allow(clippy::too_many_arguments)]
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
        env.storage()
            .instance()
            .set(&DataKey::BatchCapEnforced, &true);

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

    /// Admin enables or disables the aggregate batch-cap check.
    pub fn set_batch_cap_enforcement(env: Env, enabled: bool) -> Result<(), FactoryError> {
        require_admin(&env)?;

        env.storage()
            .instance()
            .set(&DataKey::BatchCapEnforced, &enabled);
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
            batch_cap_enforced: env
                .storage()
                .instance()
                .get(&DataKey::BatchCapEnforced)
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
        memo: Option<soroban_sdk::Bytes>,
        kind: fluxora_stream::StreamKind,
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

        if let Some(ref m) = memo {
            if m.len() as usize > fluxora_stream::MAX_MEMO_BYTES {
                return Err(FactoryError::InvalidMemo);
            }
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

        let stream_id = stream_client.create_stream(
            &sender,
            &recipient,
            &deposit_amount,
            &rate_per_second,
            &start_time,
            &cliff_time,
            &end_time,
            &withdraw_dust_threshold,
            &memo,
            &kind,
        );

        Ok(stream_id)
    }

    /// Create multiple streams in one atomic factory-wrapped transaction.
    pub fn create_streams(
        env: Env,
        sender: Address,
        streams: Vec<fluxora_stream::CreateStreamParams>,
    ) -> Result<Vec<u64>, FactoryError> {
        sender.require_auth();

        let max_deposit: i128 = env
            .storage()
            .instance()
            .get(&DataKey::MaxDepositCap)
            .ok_or(FactoryError::NotInitialized)?;

        let min_duration: u64 = env
            .storage()
            .instance()
            .get(&DataKey::MinDuration)
            .ok_or(FactoryError::NotInitialized)?;

        let enforce_batch_cap: bool = env
            .storage()
            .instance()
            .get(&DataKey::BatchCapEnforced)
            .ok_or(FactoryError::NotInitialized)?;

        let stream_contract: Address = env
            .storage()
            .instance()
            .get(&DataKey::StreamContract)
            .ok_or(FactoryError::NotInitialized)?;

        let mut total_deposit: i128 = 0;
        for params in streams.iter() {
            let is_allowed: bool = env
                .storage()
                .persistent()
                .get(&DataKey::Allowlist(params.recipient.clone()))
                .unwrap_or(false);
            if !is_allowed {
                return Err(FactoryError::RecipientNotAllowlisted);
            }

            if params.deposit_amount > max_deposit {
                return Err(FactoryError::DepositExceedsCap);
            }

            if params.start_time >= params.end_time {
                return Err(FactoryError::InvalidTimeRange);
            }
            if params.cliff_time < params.start_time || params.cliff_time > params.end_time {
                return Err(FactoryError::InvalidCliff);
            }

            let duration = params.end_time - params.start_time;
            if duration < min_duration {
                return Err(FactoryError::DurationTooShort);
            }

            if let Some(ref m) = params.memo {
                if m.len() as usize > fluxora_stream::MAX_MEMO_BYTES {
                    return Err(FactoryError::InvalidMemo);
                }
            }

            if enforce_batch_cap {
                total_deposit = total_deposit
                    .checked_add(params.deposit_amount)
                    .ok_or(FactoryError::DepositExceedsCap)?;
                if total_deposit > max_deposit {
                    return Err(FactoryError::DepositExceedsCap);
                }
            }
        }

        if streams.is_empty() {
            return Ok(Vec::new(&env));
        }

        let stream_client = FluxoraStreamClient::new(&env, &stream_contract);
        let mut wrapped_streams = Vec::new(&env);
        for params in streams.iter() {
            wrapped_streams.push_back(params.clone());
        }

        let created_ids = stream_client.create_streams(&sender, &wrapped_streams);
        Ok(created_ids)
    }
}
