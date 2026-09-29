#![no_std]
//! # Fluxora factory — the admin policy layer
//!
//! [`FluxoraStream`](https://github.com/Fluxora-Org/Fluxora-Contracts) is
//! deliberately a primitive: no admin, no pause switch, no fees, no upgrade
//! path. Those are exactly the powers that make a streaming primitive unsafe to
//! build on, so the contract refuses to carry them. Everything an *operator*
//! needs instead lives one layer up, here:
//!
//! * who may receive (an allowlist),
//! * how large a single deposit may be (a cap),
//! * how short a schedule may be (a minimum duration),
//! * how fast and how slow a stream may pay out (optional rate bounds),
//! * whether new streams are being accepted at all (the pause flag).
//!
//! This contract owns no tokens and moves no funds. It stores a small set of
//! policy values and exposes admin setters plus read-only views over them.
//!
//! ## Everything is instance storage
//!
//! The whole configuration lives in the contract's **instance** entry, with the
//! single exception of the per-address allowlist, which is persistent because it
//! is keyed per recipient. The instance entry is inspected on every state
//! change and pinned to `env.storage().max_ttl()`, the same policy the stream
//! contract applies to its own instance entry: it is tiny, it carries the
//! admin, and letting it archive would brick every admin operation. An
//! actively-administered factory therefore never expires, and an idle one is
//! repaired — entry and rent together — by the first setter that touches it.
//!
//! Read-only views ([`FluxoraFactory::get_factory_config`],
//! [`FluxoraFactory::is_factory_paused`], [`FluxoraFactory::is_allowlisted`])
//! deliberately do **not** bump TTL. They are called through simulation by SDKs
//! and dashboards, and a view that paid rent would be an observable behaviour
//! change for a caller who only wanted to read. This mirrors the stream
//! contract's rule that views never extend TTL (#1686).
//!
//! ## One chokepoint: [`load_policy`]
//!
//! Every policy axis is read through [`load_policy`] rather than by poking
//! individual storage keys. That keeps every caller on the same, complete policy
//! set: adding an axis here does not require touching each creation path, and no
//! constraint can be silently skipped by a path that forgot about it. The
//! factory's own creation guard ([`FluxoraFactory::assert_creation_allowed`])
//! goes through it for exactly that reason.
//!
//! ## Immutability posture
//!
//! There is no upgrade entry point. A change to the policy *set* is a new
//! deployment; individual *values* are mutable by the admin, which is the point.
//! A cap that could never be raised would make the factory useless the first
//! time it was set too low.

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
/// The ceiling is deliberately generous — 100 years of 365-day years — so every
/// plausible treasury vesting schedule stays valid, while a fat-fingered or
/// malformed policy can never make factory-routed creation impractical forever.
pub const MAX_MIN_DURATION_SECONDS: u64 = 100 * 365 * 24 * 60 * 60;

/// Smallest accepted deposit cap.
///
/// A cap of zero would reject every stream, so it is treated as a
/// misconfiguration rather than as a valid policy.
pub const MIN_DEPOSIT_CAP: i128 = 1;

/// Every failure mode of the factory is a typed error.
///
/// Discriminants are part of the public ABI. Never renumber an existing variant;
/// only append. They live in their own numeric space, separate from the stream
/// contract's `Error` table — a client always knows which contract it called,
/// so the same number means different things in the two contracts.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum FactoryError {
    /// `init` was called on an already-initialised factory.
    AlreadyInitialized = 1,
    /// An admin entry point — or a required policy read — ran before `init`.
    NotInitialized = 2,
    /// The caller is not the stored admin. The entries below call
    /// `require_auth` on the freshly-read admin, which traps before this
    /// variant could be constructed; it is reserved as the typed encoding of
    /// the same condition for future entry points.
    Unauthorized = 3,
    /// `init` / `set_cap` received a cap below [`MIN_DEPOSIT_CAP`].
    InvalidCap = 4,
    /// `init` / `set_min_duration` received a duration above
    /// [`MAX_MIN_DURATION_SECONDS`].
    InvalidMinDuration = 5,
    /// `set_rate_bounds` received a negative bound, or a `min` above `max`.
    InvalidRateBounds = 6,
    /// The factory-level creation pause is on, so a factory-mediated creation
    /// was refused. Returned by
    /// [`FluxoraFactory::assert_creation_allowed`].
    FactoryPaused = 7,
}

/// Storage keys.
///
/// Every key here is **instance** storage except [`DataKey::Allowlist`], which
/// is per-address and therefore persistent. The instance entry is the entire
/// configuration and is bumped to the network maximum on every state change —
/// see the crate docs.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DataKey {
    /// Instance. The address authorised to call the admin setters.
    Admin,
    /// Instance. Downstream stream contract factory-mediated creations are
    /// forwarded to.
    StreamContract,
    /// Instance. Maximum per-stream deposit accepted by the factory.
    MaxDepositCap,
    /// Instance. Minimum stream duration accepted by the factory, in seconds.
    MinDuration,
    /// Instance. Whether the aggregate batch-cap check is enforced.
    BatchCapEnforced,
    /// Instance. Factory-level pause flag. Missing means `false`.
    CreationPaused,
    /// Instance. Optional inclusive lower bound on the rate per second. An
    /// absent key means unrestricted; see `set_rate_bounds` for why absence,
    /// not a stored `None`, is the representation.
    MinRatePerSecond,
    /// Instance. Optional inclusive upper bound on the rate per second. An
    /// absent key means unrestricted; see `set_rate_bounds`.
    MaxRatePerSecond,
    /// Persistent. One entry per allowlisted recipient.
    Allowlist(Address),
}

/// The policy axes a creation path consults, loaded in one pass.
///
/// Loading through [`load_policy`] rather than reading individual keys keeps
/// every caller on the same, complete policy set: adding a field here does not
/// require touching each creation path, and no constraint can be silently
/// skipped by a new one.
///
/// Both rate bounds are `Option<i128>`. `None` means that side of the interval
/// is unrestricted; when both are `Some` the interval is inclusive.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FactoryPolicy {
    /// Downstream stream contract factory-mediated creations are forwarded to.
    pub stream_contract: Address,
    /// Maximum per-stream deposit accepted.
    pub max_deposit: i128,
    /// Minimum stream duration accepted, in seconds.
    pub min_duration: u64,
    /// Whether the aggregate batch-cap check is enforced.
    pub batch_cap_enforced: bool,
    /// Whether factory-mediated creation is currently paused.
    pub creation_paused: bool,
    /// Optional inclusive lower bound on the rate per second.
    pub min_rate_per_second: Option<i128>,
    /// Optional inclusive upper bound on the rate per second.
    pub max_rate_per_second: Option<i128>,
}

/// Read-only snapshot of the complete factory configuration.
///
/// Mirrors every [`FactoryPolicy`] field plus [`DataKey::Admin`], so one
/// [`FluxoraFactory::get_factory_config`] call reconstructs the effective
/// configuration without extra probes for the admin or the pause flag.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FactoryConfig {
    /// Address authorised to call the admin setters.
    pub admin: Address,
    /// Downstream stream contract factory-mediated creations are forwarded to.
    pub stream_contract: Address,
    /// Maximum per-stream deposit accepted.
    pub max_deposit: i128,
    /// Minimum stream duration accepted, in seconds.
    pub min_duration: u64,
    /// Whether the aggregate batch-cap check is enforced.
    pub batch_cap_enforced: bool,
    /// Whether factory-mediated creation is currently paused.
    pub creation_paused: bool,
    /// Optional inclusive lower bound on the rate per second.
    pub min_rate_per_second: Option<i128>,
    /// Optional inclusive upper bound on the rate per second.
    pub max_rate_per_second: Option<i128>,
}

/// Read the complete factory policy from instance storage in a single pass.
///
/// Every creation path must obtain its policy through this helper, so that no
/// factory-level constraint can be silently skipped.
///
/// # Errors
///
/// Returns [`FactoryError::NotInitialized`] when any **required** field is
/// absent. The required fields are `stream_contract`, `max_deposit`,
/// `min_duration` and `batch_cap_enforced`, all of which
/// [`FluxoraFactory::init`] writes unconditionally. The optional fields fall
/// back to their permissive defaults: `creation_paused` → `false`, and both
/// rate bounds → `None`.
///
/// # Context
///
/// Like every storage read, this must run with the factory as the current
/// contract — inside a contract invocation, or under `env.as_contract`.
pub fn load_policy(env: &Env) -> Result<FactoryPolicy, FactoryError> {
    let instance = env.storage().instance();

    let stream_contract: Address = instance
        .get(&DataKey::StreamContract)
        .ok_or(FactoryError::NotInitialized)?;
    let max_deposit: i128 = instance
        .get(&DataKey::MaxDepositCap)
        .ok_or(FactoryError::NotInitialized)?;
    let min_duration: u64 = instance
        .get(&DataKey::MinDuration)
        .ok_or(FactoryError::NotInitialized)?;
    let batch_cap_enforced: bool = instance
        .get(&DataKey::BatchCapEnforced)
        .ok_or(FactoryError::NotInitialized)?;

    // Optional axes: absent means "permissive", never "zero". Reading a
    // missing rate bound as `0` would silently reject every stream with
    // RateBelowMin — the failure mode the `_defaults_rate_bounds_to_none`
    // regression test exists to catch.
    let creation_paused: bool = instance.get(&DataKey::CreationPaused).unwrap_or(false);
    let min_rate_per_second: Option<i128> = instance.get(&DataKey::MinRatePerSecond);
    let max_rate_per_second: Option<i128> = instance.get(&DataKey::MaxRatePerSecond);

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

/// Load and authorise the stored admin, or fail with the pre-init error.
///
/// This is the single authorisation chokepoint for admin-only entry points, and
/// the ordering is deliberate: the stored admin is read *before* `require_auth`
/// is called, so a setter invoked before `init` reports
/// [`FactoryError::NotInitialized`] rather than an authentication failure. There
/// is no admin to authenticate against yet, and telling a caller "init first" is
/// more useful than an opaque auth trap.
///
/// The read is fresh on every call — nothing is cached or memoised — so a
/// rotation takes effect immediately, including later in the same ledger.
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
/// contract's `extend_instance`: the entry is extended to
/// `env.storage().max_ttl()` on every state-changing call, so an
/// actively-administered factory never expires and an idle one is repaired by
/// the first setter that touches it.
///
/// No authorisation is required. This only extends rent on storage the contract
/// already owns and cannot change any value.
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

/// Reject a rate-bound pair that is negative, or inverted.
///
/// `None` on either side means "unrestricted"; only a pair that is fully
/// specified can be inverted. Validating before touching storage is what keeps
/// a rejected call from leaving a half-applied interval behind.
fn validate_rate_bounds(
    min_rate: Option<i128>,
    max_rate: Option<i128>,
) -> Result<(), FactoryError> {
    if let Some(min) = min_rate {
        if min < 0 {
            return Err(FactoryError::InvalidRateBounds);
        }
    }
    if let Some(max) = max_rate {
        if max < 0 {
            return Err(FactoryError::InvalidRateBounds);
        }
    }
    if let (Some(min), Some(max)) = (min_rate, max_rate) {
        if min > max {
            return Err(FactoryError::InvalidRateBounds);
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Events
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

/// Emitted when the rate-bounds pair is replaced.
#[contractevent]
pub struct RateBoundsUpdated {
    pub min_rate_per_second: Option<i128>,
    pub max_rate_per_second: Option<i128>,
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
    /// The supplied `admin` must authorise the call. That is what stops an
    /// unrelated caller from front-running bootstrap and seeding the factory
    /// with an admin address they do not control.
    ///
    /// `stream_contract` is stored verbatim and is **not** validated on chain:
    /// the factory holds no token and makes no cross-contract call, so there is
    /// nothing a smoke check could protect. Pointing the factory at the wrong
    /// stream contract is a configuration error, visible to off-chain tooling,
    /// not a funds risk inside this contract.
    ///
    /// Accepted policy ranges:
    ///
    /// * `max_deposit`: `MIN_DEPOSIT_CAP..=i128::MAX`
    ///   ([`FactoryError::InvalidCap`] otherwise).
    /// * `min_duration`: `0..=MAX_MIN_DURATION_SECONDS` seconds
    ///   ([`FactoryError::InvalidMinDuration`] otherwise).
    ///
    /// Batch-cap enforcement defaults to `true`; the creation pause defaults to
    /// `false`; both rate bounds default to `None`.
    ///
    /// # Errors
    ///
    /// * [`FactoryError::AlreadyInitialized`] if `init` already succeeded.
    /// * [`FactoryError::InvalidCap`] / [`FactoryError::InvalidMinDuration`] for
    ///   out-of-range policy values.
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

        let instance = env.storage().instance();
        instance.set(&DataKey::Admin, &admin);
        instance.set(&DataKey::StreamContract, &stream_contract);
        instance.set(&DataKey::MaxDepositCap, &max_deposit);
        instance.set(&DataKey::MinDuration, &min_duration);
        instance.set(&DataKey::BatchCapEnforced, &true);
        // CreationPaused is intentionally not written: a missing key reads as
        // `false`, so "never paused" needs no storage slot and no rent.

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
    /// safe no-op, because the key is simply absent.
    pub fn set_allowlist(env: Env, recipient: Address, allowed: bool) -> Result<(), FactoryError> {
        require_admin(&env)?;

        let key = DataKey::Allowlist(recipient.clone());
        if allowed {
            env.storage().persistent().set(&key, &true);
            // Give the durable entry the same maximum window as instance
            // storage, so an allowlisted recipient does not silently drop off
            // the list while idle.
            let max = env.storage().max_ttl();
            env.storage().persistent().extend_ttl(&key, max, max);
        } else {
            env.storage().persistent().remove(&key);
        }

        // The instance entry was read to authorise this call, so keep it fresh
        // on allowlist writes too.
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

    /// Replace the optional inclusive rate-per-second bounds. Requires admin
    /// auth.
    ///
    /// The pair **replaces** both bounds atomically rather than merging with
    /// what is stored, so `None` on a side means "that side is unrestricted".
    /// Passing `None` for both is therefore how the bounds are cleared, and
    /// there is no way to half-update the interval by mistake.
    ///
    /// Both bounds must be non-negative, and when both are supplied `min <= max`
    /// must hold; otherwise the call returns
    /// [`FactoryError::InvalidRateBounds`] without writing either bound.
    ///
    /// # Why `None` removes the key
    ///
    /// An unrestricted side is represented by the **absence** of its key, not by
    /// storing a `None` sentinel. A stored `Option<i128>::None` encodes to
    /// `Void`, and this SDK's decoder does not accept `Void` for `Option<i128>`
    /// — so writing the sentinel would make the very next `load_policy` fail
    /// with an opaque host type error. Absence is also the representation
    /// [`load_policy`] already documents for a never-set bound, so both the
    /// never-set and the cleared state read as `None`.
    ///
    /// # Errors
    ///
    /// * [`FactoryError::NotInitialized`] — before `init`.
    /// * [`FactoryError::InvalidRateBounds`] — a negative bound, or `min > max`.
    pub fn set_rate_bounds(
        env: Env,
        min_rate: Option<i128>,
        max_rate: Option<i128>,
    ) -> Result<(), FactoryError> {
        require_admin(&env)?;

        // Validate before touching storage so a rejected call can never leave a
        // half-applied interval behind.
        validate_rate_bounds(min_rate, max_rate)?;

        let instance = env.storage().instance();
        // `None` clears the side by removing the key; see the doc comment above.
        // Both sides are written (or removed) together, so the interval can
        // never be observed half-applied.
        match min_rate {
            Some(value) => instance.set(&DataKey::MinRatePerSecond, &value),
            None => instance.remove(&DataKey::MinRatePerSecond),
        }
        match max_rate {
            Some(value) => instance.set(&DataKey::MaxRatePerSecond, &value),
            None => instance.remove(&DataKey::MaxRatePerSecond),
        }
        bump_instance(&env);

        RateBoundsUpdated {
            min_rate_per_second: min_rate,
            max_rate_per_second: max_rate,
        }
        .publish(&env);
        Ok(())
    }

    /// Toggle the factory-level creation pause. Requires admin auth.
    ///
    /// When `paused` is `true`, a factory-mediated creation is refused with
    /// [`FactoryError::FactoryPaused`] by
    /// [`FluxoraFactory::assert_creation_allowed`], before any other policy
    /// check runs. Flipping this flag touches nothing but the pause bit: it
    /// cannot move funds, cannot alter the schedule or status of any stream
    /// already issued by the downstream contract, and cannot change any other
    /// policy axis.
    pub fn set_factory_paused(env: Env, paused: bool) -> Result<(), FactoryError> {
        require_admin(&env)?;

        env.storage()
            .instance()
            .set(&DataKey::CreationPaused, &paused);
        bump_instance(&env);

        FactoryPauseUpdated { paused }.publish(&env);
        Ok(())
    }

    /// Refuse a factory-mediated creation while the pause is on.
    ///
    /// This is the pause chokepoint: it loads the full policy through
    /// [`load_policy`] and reports the pause as a **named** error,
    /// [`FactoryError::FactoryPaused`], so an integrator can branch on the
    /// reason instead of parsing an opaque trap. Reading the policy rather than
    /// the single `CreationPaused` key is deliberate — it keeps the pause on the
    /// same, complete policy set every other constraint comes from, and it
    /// fails with [`FactoryError::NotInitialized`] if the factory was never
    /// initialised.
    ///
    /// Callers are creation paths; the function moves no funds and writes
    /// nothing.
    ///
    /// # Errors
    ///
    /// * [`FactoryError::NotInitialized`] — before `init`.
    /// * [`FactoryError::FactoryPaused`] — the admin has paused creation.
    pub fn assert_creation_allowed(env: Env) -> Result<(), FactoryError> {
        let policy = load_policy(&env)?;
        if policy.creation_paused {
            return Err(FactoryError::FactoryPaused);
        }
        Ok(())
    }

    // ---------------------------------------------------------------------
    // Views — read-only, no TTL bump
    // ---------------------------------------------------------------------

    /// Whether factory-mediated creation is currently paused.
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
    /// [`FactoryError::NotInitialized`] if the factory has not been initialised.
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
