//! Fluxora Factory contract.
//!
//! Manages deployment policy for the Fluxora stream contract: admin rotation,
//! deposit cap, minimum duration, token allowlist, pause, rate bounds, and
//! batch-cap enforcement.
//!
//! ## Storage layout
//!
//! All policy fields live in **instance** storage so every setter bumps the
//! same TTL in one call. The allowlist uses **persistent** storage keyed by
//! address, so individual entries can be removed without affecting the rest of
//! the policy.
//!
//! ## Initialisation gate
//!
//! Every public entrypoint that reads or writes policy goes through
//! [`load_policy`] (or the admin-load helper), which returns
//! [`FactoryError::NotInitialized`] if the `Admin` key is absent. This makes
//! "not yet initialised" a typed, recoverable error rather than a panic.

#![no_std]

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, symbol_short, Address, Env,
};

// ---------------------------------------------------------------------------
// TTL constants (mirrors the stream contract's conservative ledger estimate)
// ---------------------------------------------------------------------------

/// Nominal seconds per ledger (conservative; errs toward keeping entries alive
/// longer). Matches `contracts/stream/src/storage.rs`.
const SECONDS_PER_LEDGER: u32 = 5;

/// The instance entry is always kept at the network maximum — it carries the
/// admin address and the policy that every `create_stream` call reads. If it
/// archived the factory would become permanently inaccessible.
const INSTANCE_BUMP_TARGET: u32 = u32::MAX;
const INSTANCE_BUMP_THRESHOLD: u32 = u32::MAX / 2;

/// Allowlist entries use persistent storage. Keep them alive for ~30 days by
/// default; a keeper or admin interaction will re-extend them.
const ALLOWLIST_TTL_LEDGERS: u32 = 30 * 24 * 3600 / SECONDS_PER_LEDGER; // ~518 400
const ALLOWLIST_TTL_THRESHOLD: u32 = ALLOWLIST_TTL_LEDGERS / 2;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Typed errors for the factory contract.
///
/// Discriminants are ABI-stable: never renumber, only append.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum FactoryError {
    /// A view or setter was called before [`FluxoraFactory::init`].
    NotInitialized = 1,
    /// [`FluxoraFactory::init`] was called on an already-initialised contract.
    AlreadyInitialized = 2,
}

// ---------------------------------------------------------------------------
// Storage keys
// ---------------------------------------------------------------------------

#[contracttype]
#[derive(Clone)]
enum DataKey {
    /// `Address` — the current admin.
    Admin,
    /// `Address` — the stream contract this factory deploys against.
    StreamContract,
    /// `i128` — maximum deposit accepted by `create_stream`.
    MaxDeposit,
    /// `u64` — minimum duration in seconds accepted by `create_stream`.
    MinDuration,
    /// `bool` — whether `create_stream` is globally paused.
    CreationPaused,
    /// `bool` — whether the per-stream cap is enforced on batch creation.
    BatchCapEnforced,
    /// `Option<i128>` — minimum rate per second (None = no lower bound).
    MinRatePerSecond,
    /// `Option<i128>` — maximum rate per second (None = no upper bound).
    MaxRatePerSecond,
    /// Per-address allowlist entry (persistent storage).
    Allowlisted(Address),
}

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Snapshot of every policy axis, returned by [`load_policy`] and
/// [`FluxoraFactory::get_factory_config`].
///
/// `PartialEq` / `Clone` are derived so callers can do snapshot assertions
/// (see `test_load_policy_equality_is_struct_equality`).
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FactoryConfig {
    pub admin: Address,
    pub stream_contract: Address,
    pub max_deposit: i128,
    pub min_duration: u64,
    pub batch_cap_enforced: bool,
}

/// Full policy view used internally and by the `load_policy` helper.
///
/// Includes the optional rate bounds that are not part of [`FactoryConfig`]
/// but that `create_stream` validation needs.
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

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Extend the instance entry to the network maximum.
///
/// Called on every mutating path so the admin key and all instance-stored
/// policy fields are always funded to the longest possible TTL.
fn bump_instance(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(INSTANCE_BUMP_THRESHOLD, INSTANCE_BUMP_TARGET);
}

/// Load the admin from instance storage, returning [`FactoryError::NotInitialized`]
/// if the `Admin` key is absent (i.e. `init` has never been called).
fn require_initialized(env: &Env) -> Result<Address, FactoryError> {
    env.storage()
        .instance()
        .get::<DataKey, Address>(&DataKey::Admin)
        .ok_or(FactoryError::NotInitialized)
}

/// Load the admin and require that the caller has provided its authorisation.
///
/// Returns [`FactoryError::NotInitialized`] if `init` has not been called.
/// Auth enforcement is done via [`Address::require_auth`]; if the signer does
/// not match, the host panics (the standard Soroban auth-failure path).
fn require_admin(env: &Env) -> Result<(), FactoryError> {
    let admin = require_initialized(env)?;
    admin.require_auth();
    Ok(())
}

// ---------------------------------------------------------------------------
// Public helper — exported so tests can import it directly
// ---------------------------------------------------------------------------

/// Load the full [`FactoryPolicy`] from instance storage.
///
/// Returns [`FactoryError::NotInitialized`] if [`FluxoraFactory::init`] has
/// not been called. Intended for use by downstream contracts (e.g. the stream
/// factory entry point) and by tests that want a single-call policy snapshot.
///
/// This does **not** bump TTL: it is a pure read, intended for simulation and
/// for contexts where the caller will bump TTL themselves.
pub fn load_policy(env: &Env) -> Result<FactoryPolicy, FactoryError> {
    let storage = env.storage().instance();

    // The stream_contract key is written during init; its absence means the
    // contract has never been initialised.
    let stream_contract = storage
        .get::<DataKey, Address>(&DataKey::StreamContract)
        .ok_or(FactoryError::NotInitialized)?;

    let max_deposit = storage
        .get::<DataKey, i128>(&DataKey::MaxDeposit)
        .ok_or(FactoryError::NotInitialized)?;

    let min_duration = storage
        .get::<DataKey, u64>(&DataKey::MinDuration)
        .ok_or(FactoryError::NotInitialized)?;

    // Optional fields default to their documented values when absent.
    let batch_cap_enforced = storage
        .get::<DataKey, bool>(&DataKey::BatchCapEnforced)
        .unwrap_or(true);

    let creation_paused = storage
        .get::<DataKey, bool>(&DataKey::CreationPaused)
        .unwrap_or(false);

    let min_rate_per_second = storage
        .get::<DataKey, Option<i128>>(&DataKey::MinRatePerSecond)
        .unwrap_or(None);

    let max_rate_per_second = storage
        .get::<DataKey, Option<i128>>(&DataKey::MaxRatePerSecond)
        .unwrap_or(None);

    Ok(FactoryPolicy {
        stream_contract,
        max_deposit,
        min_duration,
        batch_cap_enforced,
        creation_paused,
        min_rate_per_second,
        max_rate_per_second,
    })
}

// ---------------------------------------------------------------------------
// Contract
// ---------------------------------------------------------------------------

#[contract]
pub struct FluxoraFactory;

#[contractimpl]
impl FluxoraFactory {
    // -----------------------------------------------------------------------
    // Initialisation
    // -----------------------------------------------------------------------

    /// Initialise the factory with the given admin, stream contract address,
    /// deposit cap, and minimum duration.
    ///
    /// May only be called once. Returns [`FactoryError::AlreadyInitialized`]
    /// on a second call.
    ///
    /// Default policy after init:
    /// - `creation_paused = false`
    /// - `batch_cap_enforced = true`
    /// - `min_rate_per_second = None`
    /// - `max_rate_per_second = None`
    pub fn init(
        env: Env,
        admin: Address,
        stream_contract: Address,
        max_deposit: i128,
        min_duration: u64,
    ) -> Result<(), FactoryError> {
        let storage = env.storage().instance();

        if storage.has(&DataKey::Admin) {
            return Err(FactoryError::AlreadyInitialized);
        }

        storage.set(&DataKey::Admin, &admin);
        storage.set(&DataKey::StreamContract, &stream_contract);
        storage.set(&DataKey::MaxDeposit, &max_deposit);
        storage.set(&DataKey::MinDuration, &min_duration);
        storage.set(&DataKey::BatchCapEnforced, &true);
        storage.set(&DataKey::CreationPaused, &false);

        bump_instance(&env);
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Views
    // -----------------------------------------------------------------------

    /// Return a snapshot of the factory's configuration.
    ///
    /// Returns [`FactoryError::NotInitialized`] if `init` has not been called.
    pub fn get_factory_config(env: Env) -> Result<FactoryConfig, FactoryError> {
        let storage = env.storage().instance();

        let admin = storage
            .get::<DataKey, Address>(&DataKey::Admin)
            .ok_or(FactoryError::NotInitialized)?;

        let stream_contract = storage
            .get::<DataKey, Address>(&DataKey::StreamContract)
            .ok_or(FactoryError::NotInitialized)?;

        let max_deposit = storage
            .get::<DataKey, i128>(&DataKey::MaxDeposit)
            .ok_or(FactoryError::NotInitialized)?;

        let min_duration = storage
            .get::<DataKey, u64>(&DataKey::MinDuration)
            .ok_or(FactoryError::NotInitialized)?;

        let batch_cap_enforced = storage
            .get::<DataKey, bool>(&DataKey::BatchCapEnforced)
            .unwrap_or(true);

        Ok(FactoryConfig {
            admin,
            stream_contract,
            max_deposit,
            min_duration,
            batch_cap_enforced,
        })
    }

    /// Return `true` if creation is currently paused.
    ///
    /// This is a read-only view; it does **not** bump the instance TTL.
    pub fn is_factory_paused(env: Env) -> bool {
        env.storage()
            .instance()
            .get::<DataKey, bool>(&DataKey::CreationPaused)
            .unwrap_or(false)
    }

    /// Return `true` if `token` is on the allowlist.
    ///
    /// Returns `false` for any address that was never added, or that was
    /// subsequently removed. This is a read-only view.
    pub fn is_allowlisted(env: Env, token: Address) -> bool {
        env.storage()
            .persistent()
            .get::<DataKey, bool>(&DataKey::Allowlisted(token))
            .unwrap_or(false)
    }

    // -----------------------------------------------------------------------
    // Admin setters
    // -----------------------------------------------------------------------

    /// Rotate the admin to `new_admin`.
    ///
    /// Requires the current admin's authorisation. Setting the admin to the
    /// same address is a no-op and does not error.
    pub fn set_admin(env: Env, new_admin: Address) -> Result<(), FactoryError> {
        require_admin(&env)?;
        env.storage()
            .instance()
            .set(&DataKey::Admin, &new_admin);
        bump_instance(&env);
        Ok(())
    }

    /// Update the stream contract address.
    ///
    /// Requires admin authorisation.
    pub fn set_stream_contract(
        env: Env,
        stream_contract: Address,
    ) -> Result<(), FactoryError> {
        require_admin(&env)?;
        env.storage()
            .instance()
            .set(&DataKey::StreamContract, &stream_contract);
        bump_instance(&env);
        Ok(())
    }

    /// Update the maximum deposit cap.
    ///
    /// Requires admin authorisation.
    pub fn set_cap(env: Env, max_deposit: i128) -> Result<(), FactoryError> {
        require_admin(&env)?;
        env.storage()
            .instance()
            .set(&DataKey::MaxDeposit, &max_deposit);
        bump_instance(&env);
        Ok(())
    }

    /// Update the minimum stream duration (in seconds).
    ///
    /// Requires admin authorisation.
    pub fn set_min_duration(env: Env, min_duration: u64) -> Result<(), FactoryError> {
        require_admin(&env)?;
        env.storage()
            .instance()
            .set(&DataKey::MinDuration, &min_duration);
        bump_instance(&env);
        Ok(())
    }

    /// Add or remove `token` from the token allowlist.
    ///
    /// - `allow = true`: add the address (idempotent).
    /// - `allow = false`: remove the address. Removing an address that was
    ///   never added is a safe no-op.
    ///
    /// Requires admin authorisation. Returns [`FactoryError::NotInitialized`]
    /// if `init` has not been called.
    pub fn set_allowlist(
        env: Env,
        token: Address,
        allow: bool,
    ) -> Result<(), FactoryError> {
        require_admin(&env)?;

        let key = DataKey::Allowlisted(token);
        if allow {
            env.storage().persistent().set(&key, &true);
            env.storage().persistent().extend_ttl(
                &key,
                ALLOWLIST_TTL_THRESHOLD,
                ALLOWLIST_TTL_LEDGERS,
            );
        } else {
            // remove is a no-op if the key is absent — exactly what the spec wants.
            env.storage().persistent().remove(&key);
        }

        bump_instance(&env);
        Ok(())
    }

    /// Pause or unpause stream creation.
    ///
    /// Requires admin authorisation.
    pub fn set_factory_paused(env: Env, paused: bool) -> Result<(), FactoryError> {
        require_admin(&env)?;
        env.storage()
            .instance()
            .set(&DataKey::CreationPaused, &paused);
        bump_instance(&env);
        Ok(())
    }

    /// Enable or disable per-stream cap enforcement on batch creation.
    ///
    /// Requires admin authorisation.
    pub fn set_batch_cap_enforcement(
        env: Env,
        enforced: bool,
    ) -> Result<(), FactoryError> {
        require_admin(&env)?;
        env.storage()
            .instance()
            .set(&DataKey::BatchCapEnforced, &enforced);
        bump_instance(&env);
        Ok(())
    }

    /// Set the minimum and/or maximum rate per second enforced at stream
    /// creation. Pass `None` to remove a bound.
    ///
    /// Requires admin authorisation.
    pub fn set_rate_bounds(
        env: Env,
        min_rate: Option<i128>,
        max_rate: Option<i128>,
    ) -> Result<(), FactoryError> {
        require_admin(&env)?;
        env.storage()
            .instance()
            .set(&DataKey::MinRatePerSecond, &min_rate);
        env.storage()
            .instance()
            .set(&DataKey::MaxRatePerSecond, &max_rate);
        bump_instance(&env);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod test {
    use super::*;
}
