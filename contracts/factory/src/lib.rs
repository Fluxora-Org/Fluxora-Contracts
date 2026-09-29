#![no_std]
//! # Fluxora factory — admin policy wrapper
//!
//! The factory is the *policy* half of Fluxora. The
//! [`FluxoraStream`](https://github.com/Fluxora-Org/Fluxora-Contracts)
//! contract is deliberately a primitive: it has no admin, no pause switch and
//! no fees, because those are exactly the powers that make a streaming
//! primitive unsafe to build on. Anything an operator needs to configure —
//! who may receive, how large a deposit may be, how short a schedule may be,
//! whether new streams are paused — lives here instead, one layer up.
//!
//! This contract owns no tokens and moves no funds. It stores a small set of
//! policy knobs, all of them in **instance storage**, and exposes admin
//! setters plus read-only views over them.
//!
//! ## Instance TTL policy
//!
//! Every state-changing entry point — [`FluxoraFactory::init`] and each admin
//! setter — calls [`bump_instance`], which pins the contract's instance entry
//! to the network maximum, `env.storage().max_ttl()`.
//!
//! This is the same policy the stream contract applies to its instance entry
//! (`fluxora_stream::storage::extend_instance`): instance storage is tiny and
//! holds the whole admin configuration, so it is kept at maximum rent on every
//! mutation rather than being allowed to decay toward some watermark. An
//! actively-administered factory therefore never lets its configuration
//! archive, and an idle factory is brought back to a full TTL window by the
//! first setter call after the gap — the setter reads and writes instance
//! storage, so the bump repairs the entry as part of the same transaction.
//!
//! Read-only views ([`FluxoraFactory::get_factory_config`],
//! [`FluxoraFactory::is_factory_paused`], [`FluxoraFactory::is_allowlisted`])
//! deliberately do **not** bump TTL. They can run in simulation, and a view
//! that paid rent would be an observable behaviour change for callers who only
//! want to read state. This mirrors the stream contract's rule that views never
//! extend TTL.
//!
//! ## Immutability posture
//!
//! There is no upgrade entry point. A change to the policy *set* itself (not
//! its values) is a new deployment. Individual values are mutable by the
//! admin, which is the point: a cap that could never be raised would make the
//! factory useless the first time it was set too low.

#[cfg(all(target_family = "wasm", not(target_os = "none")))]
compile_error!(
    "Fluxora production WASM must be built for wasm32v1-none; wasm32-unknown-unknown can emit unsupported features."
);
#[cfg(all(target_family = "wasm", debug_assertions))]
compile_error!("Fluxora production WASM must be built without debug assertions; use --release.");
#[cfg(all(target_family = "wasm", feature = "testutils"))]
compile_error!("Fluxora production WASM must not enable the testutils feature.");

// The test suite runs against the host with `std` available; the contract
// itself is strictly `no_std`.
#[cfg(test)]
extern crate std;

use soroban_sdk::{
    contract, contracterror, contractevent, contractimpl, contracttype, Address, Env,
};

/// Largest accepted [`FluxoraFactory::set_min_duration`] value, in seconds.
///
/// The ceiling is deliberately generous (100 years of 365-day years) so normal
/// treasury vesting schedules stay valid, while a fat-fingered or malformed
/// policy can never make factory-routed stream creation impractical forever.
pub const MAX_MIN_DURATION_SECONDS: u64 = 100 * 365 * 24 * 60 * 60;

/// Smallest accepted deposit cap.
///
/// A cap of zero would reject every stream, so it is treated as a
/// misconfiguration rather than a valid policy.
pub const MIN_DEPOSIT_CAP: i128 = 1;

/// Every failure mode of the factory is a typed error.
///
/// Discriminants are part of the public ABI. Never renumber an existing
/// variant; only append. They are intentionally distinct from the stream
/// contract's `Error` discriminants in intent, though the numeric space is
/// shared across the two ABI tables — a client always knows which contract it
/// called, so the same number means different things in different contracts.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum FactoryError {
    /// `init` was called on an already-initialized factory.
    AlreadyInitialized = 1,
    /// An admin entry point (or a required config read) ran before `init`.
    NotInitialized = 2,
    /// The caller is not the stored admin. Currently `require_auth` panics
    /// before this variant is constructed, so it is reserved as the typed
    /// encoding of the same condition for future entry points.
    Unauthorized = 3,
    /// `set_cap` (or `init`) received a cap below [`MIN_DEPOSIT_CAP`].
    InvalidCap = 4,
    /// `set_min_duration` (or `init`) received a duration above
    /// [`MAX_MIN_DURATION_SECONDS`].
    InvalidMinDuration = 5,
    /// `set_rate_bounds` received a negative bound, or `min > max`.
    InvalidRateBounds = 6,
}

/// Storage keys.
///
/// Every key here lives in **instance** storage except
/// [`DataKey::Allowlist`], which is per-address and therefore persistent. The
/// instance entry is the contract's entire configuration and is bumped to the
/// network maximum on every state change — see the crate docs.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DataKey {
    /// Instance storage. The address authorised to call the setters.
    Admin,
    /// Instance storage. Address of the downstream stream contract that
    /// factory-routed creations are forwarded to.
    StreamContract,
    /// Instance storage. Maximum per-stream deposit accepted by the factory.
    MaxDepositCap,
    /// Instance storage. Minimum stream duration accepted by the factory, in
    /// seconds.
    MinDuration,
    /// Instance storage. Whether the aggregate batch-cap check is enforced.
    BatchCapEnforced,
    /// Instance storage. Factory-level pause flag. Missing means `false`.
    CreationPaused,
    /// Instance storage. Optional inclusive lower bound on rate per second.
    MinRatePerSecond,
    /// Instance storage. Optional inclusive upper bound on rate per second.
    MaxRatePerSecond,
    /// Persistent storage. One entry per allowlisted recipient.
    Allowlist(Address),
}

/// Read-only snapshot of the complete factory configuration.
///
/// Mirrors every [`FactoryPolicy`] field plus [`DataKey::Admin`], so a single
/// [`FluxoraFactory::get_factory_config`] call reconstructs the effective
/// configuration without extra probes for the admin or the pause flag.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FactoryConfig {
    /// Address authorised to call the admin setters.
    pub admin: Address,
    /// Downstream stream contract this factory routes creations to.
    pub stream_contract: Address,
    /// Maximum per-stream deposit accepted.
    pub max_deposit: i128,
    /// Minimum stream duration accepted, in seconds.
    pub min_duration: u64,
    /// Whether the aggregate batch-cap check is enforced.
    pub batch_cap_enforced: bool,
    /// Whether factory-routed stream creation is currently paused.
    pub creation_paused: bool,
    /// Optional inclusive lower bound on rate per second.
    pub min_rate_per_second: Option<i128>,
    /// Optional inclusive upper bound on rate per second.
    pub max_rate_per_second: Option<i128>,
}

/// The policy axes a creation path consults, loaded in one pass.
///
/// Loading through [`load_policy`] rather than reading individual keys keeps
/// every caller on the same, complete policy set: adding a field here does not
/// require touching each creation path, and no constraint can be silently
/// skipped by a new one.
///
/// Both rate bounds are `Option<i128>`. `None` means the corresponding side of
/// the interval is unbounded (permissive); when both are `Some` they are
/// inclusive.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FactoryPolicy {
    /// Downstream stream contract this factory routes creations to.
    pub stream_contract: Address,
    /// Maximum per-stream deposit accepted.
    pub max_deposit: i128,
    /// Minimum stream duration accepted, in seconds.
    pub min_duration: u64,
    /// Whether the aggregate batch-cap check is enforced.
    pub batch_cap_enforced: bool,
    /// Whether factory-routed stream creation is currently paused.
    pub creation_paused: bool,
    /// Optional inclusive lower bound on rate per second.
    pub min_rate_per_second: Option<i128>,
    /// Optional inclusive upper bound on rate per second.
    pub max_rate_per_second: Option<i128>,
}

/// Read the complete factory policy from instance storage in a single pass.
///
/// Every creation path must obtain its policy through this helper so that no
/// factory-level constraint can be silently skipped.
///
/// # Errors
///
/// Returns [`FactoryError::NotInitialized`] when any **required** field is
/// absent. Required fields are `stream_contract`, `max_deposit`,
/// `min_duration` and `batch_cap_enforced`, all of which
/// [`FluxoraFactory::init`] writes unconditionally. Optional fields fall back
/// to their permissive defaults: `creation_paused` → `false`, and both rate
/// bounds → `None`.
///
/// # Context
///
/// Like every storage read, this must run with the factory as the current
/// contract (i.e. inside a contract invocation or `env.as_contract`).
pub fn load_policy(env: &Env) -> Result<FactoryPolicy, FactoryError> {
    let stream_contract: Address = env
        .storage()
        .instance()
        .get(&DataKey::StreamContract)
        .ok_or(FactoryError::NotInitialized)?;
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
    let batch_cap_enforced: bool = env
        .storage()
        .instance()
        .get(&DataKey::BatchCapEnforced)
        .ok_or(FactoryError::NotInitialized)?;
    let creation_paused: bool = env
        .storage()
        .instance()
        .get(&DataKey::CreationPaused)
        .unwrap_or(false);
    let min_rate_per_second: Option<i128> =
        env.storage().instance().get(&DataKey::MinRatePerSecond);
    let max_rate_per_second: Option<i128> =
        env.storage().instance().get(&DataKey::MaxRatePerSecond);

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

/// Load and authorize the stored admin, or fail with the pre-init error.
///
/// This is the single authorization chokepoint for admin-only entry points, and
/// it is deliberately ordered *before* `require_auth`: a setter called before
/// `init` must report [`FactoryError::NotInitialized`] rather than an
/// authentication failure, because there is no admin to authenticate against.
fn require_admin(env: &Env) -> Result<Address, FactoryError> {
    let admin: Address = env
        .storage()
        .instance()
        .get(&DataKey::Admin)
        .ok_or(FactoryError::NotInitialized)?;
    admin.require_auth();
    Ok(admin)
}

/// Pin the instance entry to the network maximum TTL.
///
/// The factory's entire configuration lives in instance storage, so letting it
/// archive would brick every admin operation. This mirrors the stream
/// contract's `extend_instance`: the entry is always extended to
/// `env.storage().max_ttl()` on every state-changing call, so an
/// actively-administered factory never expires and an idle one is repaired by
/// the first setter that touches it.
///
/// No authorization is required: this only extends rent on storage the contract
/// already owns, it cannot change any value.
fn bump_instance(env: &Env) {
    let max = env.storage().max_ttl();
    env.storage().instance().extend_ttl(max, max);
}

/// Reject a deposit cap outside `MIN_DEPOSIT_CAP..=i128::MAX`.
fn validate_cap(max_deposit: i128) -> Result<(), FactoryError> {
    if max_deposit < MIN_DEPOSIT_CAP {
        return Err(FactoryError::InvalidCap);
    }
    Ok(())
}

/// Reject a minimum duration above [`MAX_MIN_DURATION_SECONDS`].
fn validate_min_duration(min_duration: u64) -> Result<(), FactoryError> {
    if min_duration > MAX_MIN_DURATION_SECONDS {
        return Err(FactoryError::InvalidMinDuration);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Event payloads. Topics use symbol_short! (<= 9 chars).
// ---------------------------------------------------------------------------

/// Emitted once, when the factory is first initialised.
#[contractevent]
pub struct FactoryInited {
    #[topic]
    pub admin: Address,
    pub stream_contract: Address,
    pub max_deposit: i128,
    pub min_duration: u64,
}

/// Emitted when the admin key is rotated.
#[contractevent]
pub struct FactoryAdminUpdated {
    #[topic]
    pub old_admin: Address,
    #[topic]
    pub new_admin: Address,
}

/// Emitted when the downstream stream contract address changes.
#[contractevent]
pub struct StreamContractUpdated {
    #[topic]
    pub old_contract: Address,
    #[topic]
    pub new_contract: Address,
}

/// Emitted when the aggregate batch-cap enforcement flag changes.
#[contractevent]
pub struct BatchCapEnforcementUpdated {
    pub enabled: bool,
}

/// Emitted when an address is added to or removed from the allowlist.
#[contractevent]
pub struct AllowlistUpdated {
    #[topic]
    pub recipient: Address,
    pub allowed: bool,
}

/// Emitted when the factory-level creation pause is toggled.
#[contractevent]
pub struct FactoryPauseUpdated {
    pub paused: bool,
}

#[contract]
pub struct FluxoraFactory;

#[contractimpl]
impl FluxoraFactory {
    // ---------------------------------------------------------------------
    // Lifecycle
    // ---------------------------------------------------------------------

    /// Initialise the factory. May be called exactly once.
    ///
    /// The supplied `admin` must authorize the call. This is what stops an
    /// unrelated caller from front-running bootstrap and seeding the factory
    /// with an admin address they do not control.
    ///
    /// `stream_contract` is stored verbatim and is **not** validated on chain:
    /// the factory holds no token and makes no cross-contract call, so there is
    /// nothing for a smoke check to protect. Pointing the factory at the wrong
    /// stream contract is a configuration error that is visible to off-chain
    /// tooling, not a funds risk inside this contract.
    ///
    /// Accepted policy ranges:
    ///
    /// * `max_deposit`: `MIN_DEPOSIT_CAP..=i128::MAX`
    ///   ([`FactoryError::InvalidCap`] otherwise).
    /// * `min_duration`: `0..=MAX_MIN_DURATION_SECONDS` seconds
    ///   ([`FactoryError::InvalidMinDuration`] otherwise).
    ///
    /// Batch-cap enforcement defaults to `true`; creation pause defaults to
    /// `false`; rate bounds default to `None`.
    ///
    /// # Errors
    ///
    /// * [`FactoryError::AlreadyInitialized`] if `init` already succeeded.
    /// * [`FactoryError::InvalidCap`] / [`FactoryError::InvalidMinDuration`]
    ///   for out-of-range policy values.
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
        env.storage()
            .instance()
            .set(&DataKey::BatchCapEnforced, &true);
        // CreationPaused is intentionally not written: a missing key reads as
        // `false`, so "never paused" needs no storage slot.

        // Bring the freshly written instance entry to a full TTL window.
        bump_instance(&env);

        FactoryInited {
            admin,
            stream_contract,
            max_deposit,
            min_duration,
        }
        .publish(&env);
        Ok(())
    }

    // ---------------------------------------------------------------------
    // Admin setters
    // ---------------------------------------------------------------------

    /// Rotate the admin to `new_admin`. Requires the current admin's auth.
    ///
    /// Setting the current admin as `new_admin` is a successful no-op that
    /// still refreshes the instance TTL.
    pub fn set_admin(env: Env, new_admin: Address) -> Result<(), FactoryError> {
        let old_admin = require_admin(&env)?;

        env.storage().instance().set(&DataKey::Admin, &new_admin);
        bump_instance(&env);

        FactoryAdminUpdated {
            old_admin,
            new_admin,
        }
        .publish(&env);
        Ok(())
    }

    /// Point the factory at a new downstream stream contract.
    ///
    /// Requires the admin's auth. The address is stored without an on-chain
    /// reachability check — see [`FluxoraFactory::init`] for why.
    pub fn set_stream_contract(env: Env, new_stream_contract: Address) -> Result<(), FactoryError> {
        require_admin(&env)?;

        let old_contract: Address = env
            .storage()
            .instance()
            .get(&DataKey::StreamContract)
            .ok_or(FactoryError::NotInitialized)?;

        env.storage()
            .instance()
            .set(&DataKey::StreamContract, &new_stream_contract);
        bump_instance(&env);

        StreamContractUpdated {
            old_contract,
            new_contract: new_stream_contract,
        }
        .publish(&env);
        Ok(())
    }

    /// Set the maximum per-stream deposit. Requires the admin's auth.
    ///
    /// The cap must be at least [`MIN_DEPOSIT_CAP`]; a lower value returns
    /// [`FactoryError::InvalidCap`] and leaves the stored cap unchanged.
    pub fn set_cap(env: Env, max_deposit: i128) -> Result<(), FactoryError> {
        require_admin(&env)?;
        validate_cap(max_deposit)?;

        env.storage()
            .instance()
            .set(&DataKey::MaxDepositCap, &max_deposit);
        bump_instance(&env);
        Ok(())
    }

    /// Set the minimum stream duration, in seconds. Requires the admin's auth.
    ///
    /// `0` disables any factory-level minimum. Values above
    /// [`MAX_MIN_DURATION_SECONDS`] return [`FactoryError::InvalidMinDuration`]
    /// and leave the stored policy unchanged.
    pub fn set_min_duration(env: Env, min_duration: u64) -> Result<(), FactoryError> {
        require_admin(&env)?;
        validate_min_duration(min_duration)?;

        env.storage()
            .instance()
            .set(&DataKey::MinDuration, &min_duration);
        bump_instance(&env);
        Ok(())
    }

    /// Add (`allowed == true`) or remove (`allowed == false`) a recipient from
    /// the allowlist. Requires the admin's auth.
    ///
    /// Removal is idempotent: removing an address that was never added is a
    /// safe no-op (the key is simply absent).
    pub fn set_allowlist(env: Env, recipient: Address, allowed: bool) -> Result<(), FactoryError> {
        require_admin(&env)?;

        let key = DataKey::Allowlist(recipient.clone());
        if allowed {
            env.storage().persistent().set(&key, &true);
            // Give the durable entry the same maximum window as instance
            // storage, so an allowlisted recipient does not silently fall out
            // of the list while idle.
            let max = env.storage().max_ttl();
            env.storage().persistent().extend_ttl(&key, max, max);
        } else {
            env.storage().persistent().remove(&key);
        }

        // The instance entry still changed hands (admin auth was read), so keep
        // it fresh on every allowlist write too.
        bump_instance(&env);

        AllowlistUpdated { recipient, allowed }.publish(&env);
        Ok(())
    }

    /// Enable or disable the aggregate batch-cap check. Requires admin auth.
    pub fn set_batch_cap_enforcement(env: Env, enabled: bool) -> Result<(), FactoryError> {
        require_admin(&env)?;

        env.storage()
            .instance()
            .set(&DataKey::BatchCapEnforced, &enabled);
        bump_instance(&env);

        BatchCapEnforcementUpdated { enabled }.publish(&env);
        Ok(())
    }

    /// Set optional inclusive rate-per-second bounds. Requires admin auth.
    ///
    /// `None` leaves the corresponding bound unchanged. A supplied bound must
    /// be non-negative, and the resulting `min <= max` invariant must hold;
    /// otherwise the call returns [`FactoryError::InvalidRateBounds`] without
    /// writing either bound.
    pub fn set_rate_bounds(
        env: Env,
        min_rate: Option<i128>,
        max_rate: Option<i128>,
    ) -> Result<(), FactoryError> {
        require_admin(&env)?;

        // Validate the incoming values before touching storage so a rejected
        // call never leaves a half-applied interval.
        if let Some(min_v) = min_rate {
            if min_v < 0 {
                return Err(FactoryError::InvalidRateBounds);
            }
        }
        if let Some(max_v) = max_rate {
            if max_v < 0 {
                return Err(FactoryError::InvalidRateBounds);
            }
        }

        let new_min = min_rate.or_else(|| env.storage().instance().get(&DataKey::MinRatePerSecond));
        let new_max = max_rate.or_else(|| env.storage().instance().get(&DataKey::MaxRatePerSecond));
        if let (Some(mn), Some(mx)) = (new_min, new_max) {
            if mn > mx {
                return Err(FactoryError::InvalidRateBounds);
            }
        }

        if let Some(min_v) = min_rate {
            env.storage()
                .instance()
                .set(&DataKey::MinRatePerSecond, &min_v);
        }
        if let Some(max_v) = max_rate {
            env.storage()
                .instance()
                .set(&DataKey::MaxRatePerSecond, &max_v);
        }

        bump_instance(&env);
        Ok(())
    }

    /// Toggle the factory-level creation pause. Requires admin auth.
    ///
    /// When `paused` is `true`, factory-routed creation rejects new requests
    /// before doing any other work, so an operator can halt new streams without
    /// dismantling the rest of the policy.
    pub fn set_factory_paused(env: Env, paused: bool) -> Result<(), FactoryError> {
        require_admin(&env)?;

        env.storage()
            .instance()
            .set(&DataKey::CreationPaused, &paused);
        bump_instance(&env);

        FactoryPauseUpdated { paused }.publish(&env);
        Ok(())
    }

    // ---------------------------------------------------------------------
    // Views — read-only, no TTL bump
    // ---------------------------------------------------------------------

    /// Whether factory-routed stream creation is currently paused.
    ///
    /// Permissionless view. A missing flag reads as `false`.
    pub fn is_factory_paused(env: Env) -> bool {
        env.storage()
            .instance()
            .get(&DataKey::CreationPaused)
            .unwrap_or(false)
    }

    /// Whether `recipient` is currently allowlisted.
    ///
    /// Permissionless view. A missing entry reads as `false`.
    pub fn is_allowlisted(env: Env, recipient: Address) -> bool {
        env.storage()
            .persistent()
            .get(&DataKey::Allowlist(recipient))
            .unwrap_or(false)
    }

    /// Return the complete effective factory configuration.
    ///
    /// A single call reconstructs everything [`FactoryConfig`] advertises,
    /// including the admin and the pause flag, so callers need not probe
    /// individual views.
    ///
    /// # Errors
    ///
    /// [`FactoryError::NotInitialized`] if the factory has not been
    /// initialized.
    pub fn get_factory_config(env: Env) -> Result<FactoryConfig, FactoryError> {
        let policy = load_policy(&env)?;
        let admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(FactoryError::NotInitialized)?;

        Ok(FactoryConfig {
            admin,
            stream_contract: policy.stream_contract,
            max_deposit: policy.max_deposit,
            min_duration: policy.min_duration,
            batch_cap_enforced: policy.batch_cap_enforced,
            creation_paused: policy.creation_paused,
            min_rate_per_second: policy.min_rate_per_second,
            max_rate_per_second: policy.max_rate_per_second,
        })
    }
}
