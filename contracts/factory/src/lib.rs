#![no_std]
#![allow(clippy::too_many_arguments)]

use fluxora_stream::{ContractError as StreamContractErr, FluxoraStreamClient};
use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, symbol_short, Address, Env,
};

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
    /// Factory stream creation is currently paused by admin.
    CreationPaused = 9,
    /// The downstream FluxoraStream contract rejected creation because it is paused.
    StreamContractPaused = 10,
    /// The downstream FluxoraStream contract rejected creation for a reason other than paused.
    /// This is a passthrough catch-all for unexpected downstream failures.
    StreamContractError = 11,
}

#[contracttype]
pub enum DataKey {
    Admin,
    StreamContract,
    MaxDepositCap,
    MinDuration,
    Allowlist(Address),
    /// Boolean flag: when `true`, `create_stream` rejects all new streams.
    CreationPaused,
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
        // CreationPaused defaults to false — no explicit write needed;
        // `is_factory_paused` falls back to `false` on a missing key.

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

    /// Toggle the factory-level stream creation pause.
    ///
    /// When `paused` is `true`, all calls to `create_stream` immediately return
    /// [`FactoryError::CreationPaused`] — before any policy read — allowing the
    /// admin to halt new factory-originated streams without dismantling the
    /// allowlist or other policy state.
    ///
    /// # Authorization
    /// Requires the stored admin's signature. Callers that are not the admin
    /// will have their transaction rejected by `require_auth`.
    ///
    /// # Events
    /// Emits a `factory_paused` or `factory_resumed` topic depending on the
    /// new value of `paused`.
    ///
    /// # Errors
    /// - [`FactoryError::NotInitialized`] — factory has not been initialized.
    pub fn set_factory_paused(env: Env, paused: bool) -> Result<(), FactoryError> {
        require_admin(&env)?;

        env.storage()
            .instance()
            .set(&DataKey::CreationPaused, &paused);

        // Emit a structured event so indexers and monitors can react.
        if paused {
            env.events().publish(
                (symbol_short!("factory"), symbol_short!("paused")),
                paused,
            );
        } else {
            env.events().publish(
                (symbol_short!("factory"), symbol_short!("resumed")),
                paused,
            );
        }

        Ok(())
    }

    /// Return whether factory stream creation is currently paused.
    ///
    /// This is a permissionless view — anyone may call it to check the current
    /// pause state before submitting a `create_stream` transaction.
    pub fn is_factory_paused(env: Env) -> bool {
        env.storage()
            .instance()
            .get(&DataKey::CreationPaused)
            .unwrap_or(false)
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
    ///
    /// # Guard order (checked strictly in sequence)
    /// 1. **CreationPaused** — rejects immediately, before any policy read, to
    ///    avoid leaking allowlist or cap state during an incident.
    /// 2. Allowlist check
    /// 3. Deposit cap check
    /// 4. Time-range invariants
    /// 5. Minimum-duration check
    /// 6. Cross-contract stream creation
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
        // ── Guard 1: pause check (before any policy read) ───────────────────
        // Checked first so that no allowlist or cap state is observable when
        // the factory is in emergency-pause mode.
        let paused: bool = env
            .storage()
            .instance()
            .get(&DataKey::CreationPaused)
            .unwrap_or(false);
        if paused {
            return Err(FactoryError::CreationPaused);
        }

        // ── Guard 2: allowlist ───────────────────────────────────────────────
        let is_allowed: bool = env
            .storage()
            .persistent()
            .get(&DataKey::Allowlist(recipient.clone()))
            .unwrap_or(false);
        if !is_allowed {
            return Err(FactoryError::RecipientNotAllowlisted);
        }

        // ── Guard 3: deposit cap ─────────────────────────────────────────────
        let max_deposit: i128 = env
            .storage()
            .instance()
            .get(&DataKey::MaxDepositCap)
            .ok_or(FactoryError::NotInitialized)?;
        if deposit_amount > max_deposit {
            return Err(FactoryError::DepositExceedsCap);
        }

        // ── Guard 4: time invariants ─────────────────────────────────────────
        // Mirror FluxoraStream time invariants before the cross-contract call so
        // invalid schedules return typed factory errors instead of downstream panics.
        if start_time >= end_time {
            return Err(FactoryError::InvalidTimeRange);
        }
        if cliff_time < start_time || cliff_time > end_time {
            return Err(FactoryError::InvalidCliff);
        }

        // ── Guard 5: minimum duration ────────────────────────────────────────
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

        match stream_client.try_create_stream(
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
        ) {
            Ok(Ok(stream_id)) => Ok(stream_id),
            // Recognized downstream contract error reported in the success frame.
            Ok(Err(_)) => Err(FactoryError::StreamContractError),
            Err(Ok(StreamContractErr::ContractPaused)) => Err(FactoryError::StreamContractPaused),
            Err(Ok(_)) => Err(FactoryError::StreamContractError),
            Err(Err(_)) => Err(FactoryError::StreamContractError),
        }
    }
}
