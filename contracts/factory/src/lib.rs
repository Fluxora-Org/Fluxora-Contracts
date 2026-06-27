#![no_std]
#![allow(clippy::too_many_arguments)]

use fluxora_stream::{ContractError as StreamContractErr, FluxoraStreamClient};
use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, symbol_short, vec, Address, Env, Vec,
};

/// Maximum number of stream IDs returned per page in `get_factory_streams_paginated`.
///
/// Mirrors `MAX_PAGE_SIZE` from the stream contract to keep pagination semantics
/// consistent across both contracts.
pub const MAX_PAGE_SIZE: u32 = 100;

/// Instance TTL threshold (ledgers). Below this value the entry will be extended.
/// Mirrors governance contract to keep TTL semantics consistent across contracts.
const INSTANCE_LIFETIME_THRESHOLD: u32 = 17_280;

/// Instance TTL bump target (ledgers). ~60 days at 5-second ledger close.
/// Mirrors governance contract to keep TTL semantics consistent across contracts.
const INSTANCE_BUMP_AMOUNT: u32 = 120_960;

/// Persistent TTL threshold (ledgers). Below this value the entry will be extended.
const PERSISTENT_LIFETIME_THRESHOLD: u32 = 17_280;

/// Persistent TTL bump target (ledgers). ~60 days at 5-second ledger close.
const PERSISTENT_BUMP_AMOUNT: u32 = 120_960;

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
    /// Factory stream creation is currently paused by admin.
    CreationPaused = 9,
    /// The downstream FluxoraStream contract rejected creation because it is paused.
    StreamContractPaused = 10,
    /// The downstream FluxoraStream contract rejected creation for a reason other than paused.
    /// This is a passthrough catch-all for unexpected downstream failures.
    StreamContractError = 11,
    /// Rate per second is below the configured minimum.
    RateBelowMin = 12,
    /// Rate per second exceeds the configured maximum.
    RateAboveMax = 13,
    /// The factory cap must be in the accepted range `1..=i128::MAX`.
    InvalidCap = 14,
    /// The minimum duration must be in the accepted range
    /// `0..=MAX_MIN_DURATION_SECONDS` seconds.
    InvalidMinDuration = 15,
    /// The requested memo exceeds the allowed max length.
    InvalidMemo = 16,
}

#[contracttype]
pub enum DataKey {
    Admin,
    StreamContract,
    MaxDepositCap,
    MinDuration,
    BatchCapEnforced,
    Allowlist(Address),
    /// Persistent ordered list of stream IDs created through this factory.
    FactoryStreamIds,
    /// Boolean flag: when `true`, `create_stream` rejects all new streams.
    CreationPaused,
    /// Optional lower bound on rate_per_second (inclusive). When absent, no lower bound.
    MinRatePerSecond,
    /// Optional upper bound on rate_per_second (inclusive). When absent, no upper bound.
    MaxRatePerSecond,
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

/// Bump the instance storage TTL to prevent factory config expiration.
///
/// The factory's instance entries (Admin, StreamContract, MaxDepositCap, MinDuration,
/// BatchCapEnforced, rate bounds, CreationPaused) hold the factory's entire config.
/// Letting them expire would brick all admin operations. This helper extends the TTL
/// whenever they are read or written, ensuring a busy factory never lets critical
/// config expire.
///
/// **Security note:** This operation requires no additional authorization—it only
/// extends the TTL of already-protected instance storage without mutating any config.
/// It mirrors the governance contract's TTL bump patterns.
fn bump_instance(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
}

/// Load the factory-created stream ID list from persistent storage.
fn load_stream_ids(env: &Env) -> Vec<u64> {
    env.storage()
        .persistent()
        .get(&DataKey::FactoryStreamIds)
        .unwrap_or_else(|| vec![env])
}

/// Append `stream_id` to the factory registry and bump its persistent TTL.
///
/// The TTL is bumped unconditionally on every write so that a busy factory never
/// lets the index expire.
fn append_stream_id(env: &Env, stream_id: u64) {
    let mut ids = load_stream_ids(env);
    ids.push_back(stream_id);
    env.storage()
        .persistent()
        .set(&DataKey::FactoryStreamIds, &ids);
    env.storage().persistent().extend_ttl(
        &DataKey::FactoryStreamIds,
        PERSISTENT_LIFETIME_THRESHOLD,
        PERSISTENT_BUMP_AMOUNT,
    );
}

/// Validate a factory deposit cap before storing it.
///
/// The cap must be strictly positive. A non-positive cap would make every
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
    pub batch_cap_enforced: bool,
}

// ---------------------------------------------------------------------------
// Event data structs
// Topics use symbol_short! (≤ 9 chars). Naming mirrors contracts/stream/src/lib.rs.
// ---------------------------------------------------------------------------

/// Emitted when the factory is first initialised (`fct_init`).
#[contracttype]
#[derive(Clone, Debug)]
pub struct FactoryInited {
    pub admin: Address,
    pub stream_contract: Address,
    pub max_deposit: i128,
    pub min_duration: u64,
}

/// Emitted when the factory admin is rotated (`AdminUpd`).
/// Mirrors the `AdminUpd` topic used in `FluxoraStream::set_admin`.
#[contracttype]
#[derive(Clone, Debug)]
pub struct FactoryAdminUpdated {
    pub old_admin: Address,
    pub new_admin: Address,
}

/// Emitted when the stream-contract pointer is changed (`stm_upd`).
#[contracttype]
#[derive(Clone, Debug)]
pub struct StreamContractUpdated {
    pub old_contract: Address,
    pub new_contract: Address,
}

/// Emitted when a recipient is added to or removed from the allowlist (`allow_upd`).
/// Carries enough state for an indexer to reconstruct membership without re-reading storage.
#[contracttype]
#[derive(Clone, Debug)]
pub struct AllowlistUpdated {
    pub recipient: Address,
    pub allowed: bool,
}

/// Emitted when the factory deposit cap is changed (`cap_upd`).
#[contracttype]
#[derive(Clone, Debug)]
pub struct CapUpdated {
    pub old_cap: i128,
    pub new_cap: i128,
}

/// Emitted when the factory minimum-duration policy is changed (`dur_upd`).
#[contracttype]
#[derive(Clone, Debug)]
pub struct MinDurationUpdated {
    pub old_min_duration: u64,
    pub new_min_duration: u64,
}

/// Emitted when rate-per-second bounds are updated (`rate_bnd`).
#[contracttype]
#[derive(Clone, Debug)]
pub struct RateBoundsUpdated {
    pub min_rate: Option<i128>,
    pub max_rate: Option<i128>,
}

/// Emitted when a stream is successfully created through the factory (`fct_strm`).
/// Provides enough context for indexers to attribute stream creation to a policy-gated path.
#[contracttype]
#[derive(Clone, Debug)]
pub struct FactoryStreamCreated {
    pub stream_id: u64,
    pub sender: Address,
    pub recipient: Address,
    pub deposit_amount: i128,
    pub rate_per_second: i128,
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
        env.storage()
            .instance()
            .set(&DataKey::BatchCapEnforced, &true);
        // CreationPaused defaults to false — no explicit write needed;
        // `is_factory_paused` falls back to `false` on a missing key.

        // Bump instance TTL to ensure config persists across the factory's lifetime.
        bump_instance(&env);

        env.events().publish(
            (symbol_short!("fct_init"),),
            FactoryInited {
                admin,
                stream_contract,
                max_deposit,
                min_duration,
            },
        );

        Ok(())
    }

    /// Admin updates the factory admin.
    pub fn set_admin(env: Env, new_admin: Address) -> Result<(), FactoryError> {
        let old_admin = require_admin(&env)?;

        env.storage().instance().set(&DataKey::Admin, &new_admin);

        // Bump instance TTL after successful update.
        bump_instance(&env);

        env.events().publish(
            (symbol_short!("AdminUpd"),),
            FactoryAdminUpdated { old_admin, new_admin },
        );
        Ok(())
    }

    /// Admin updates the stream contract address.
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

        // Bump instance TTL after successful update.
        bump_instance(&env);

        env.events().publish(
            (symbol_short!("stm_upd"),),
            StreamContractUpdated {
                old_contract,
                new_contract: new_stream_contract,
            },
        );
        Ok(())
    }

    /// Admin adds or removes a recipient from the allowlist.
    pub fn set_allowlist(env: Env, recipient: Address, allowed: bool) -> Result<(), FactoryError> {
        require_admin(&env)?;

        let key = DataKey::Allowlist(recipient.clone());
        if allowed {
            env.storage().persistent().set(&key, &true);
        } else {
            env.storage().persistent().remove(&key);
        }

        env.events().publish(
            (symbol_short!("allow_upd"),),
            AllowlistUpdated { recipient, allowed },
        );
        Ok(())
    }

    /// Admin updates the max deposit cap.
    ///
    /// The cap must be strictly positive; a non-positive value returns
    /// `FactoryError::InvalidCap` and leaves the stored cap unchanged.
    pub fn set_cap(env: Env, max_deposit: i128) -> Result<(), FactoryError> {
        require_admin(&env)?;
        validate_cap(max_deposit)?;

        let old_cap: i128 = env
            .storage()
            .instance()
            .get(&DataKey::MaxDepositCap)
            .unwrap_or(0);

        env.storage()
            .instance()
            .set(&DataKey::MaxDepositCap, &max_deposit);

        // Bump instance TTL after successful update.
        bump_instance(&env);

        env.events().publish(
            (symbol_short!("cap_upd"),),
            CapUpdated { old_cap, new_cap: max_deposit },
        );
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

        let old_min_duration: u64 = env
            .storage()
            .instance()
            .get(&DataKey::MinDuration)
            .unwrap_or(0);

        env.storage()
            .instance()
            .set(&DataKey::MinDuration, &min_duration);

        // Bump instance TTL after successful update.
        bump_instance(&env);

        env.events().publish(
            (symbol_short!("dur_upd"),),
            MinDurationUpdated {
                old_min_duration,
                new_min_duration: min_duration,
            },
        );
        Ok(())
    }

    /// Admin enables or disables the aggregate batch-cap check.
    pub fn set_batch_cap_enforcement(env: Env, enabled: bool) -> Result<(), FactoryError> {
        require_admin(&env)?;

        env.storage()
            .instance()
            .set(&DataKey::BatchCapEnforced, &enabled);

        // Bump instance TTL after successful update.
        bump_instance(&env);

        Ok(())
    }

    /// Admin sets optional rate-per-second bounds.
    ///
    /// Both bounds are inclusive. Unset (None) means the corresponding side of
    /// the interval is unbounded (permissive). When both are set, the invariant
    /// `0 <= min <= max` must hold.
    ///
    /// Treats `None` arguments as "leave unchanged".
    pub fn set_rate_bounds(
        env: Env,
        min_rate: Option<i128>,
        max_rate: Option<i128>,
    ) -> Result<(), FactoryError> {
        require_admin(&env)?;

        if let Some(min_v) = min_rate {
            if min_v < 0 {
                // rates are non-negative by domain convention; reject negative explicitly
                return Err(FactoryError::StreamContractError); // reuse or could add new, but keep minimal
            }
            env.storage().instance().set(&DataKey::MinRatePerSecond, &min_v);
        }
        if let Some(max_v) = max_rate {
            if max_v < 0 {
                return Err(FactoryError::StreamContractError);
            }
            env.storage().instance().set(&DataKey::MaxRatePerSecond, &max_v);
        }

        // Validate min <= max when both are present after the update
        let current_min: Option<i128> = env.storage().instance().get(&DataKey::MinRatePerSecond);
        let current_max: Option<i128> = env.storage().instance().get(&DataKey::MaxRatePerSecond);
        if let (Some(mn), Some(mx)) = (current_min, current_max) {
            if mn > mx {
                return Err(FactoryError::StreamContractError);
            }
        }

        // Bump instance TTL after successful update.
        bump_instance(&env);

        env.events().publish(
            (symbol_short!("rate_bnd"),),
            RateBoundsUpdated { min_rate, max_rate },
        );
        Ok(())
    }

    /// Toggle the factory-level stream creation pause.
    ///
    /// When `paused` is `true`, all calls to `create_stream` immediately return
    /// [`FactoryError::CreationPaused`] — before any policy read, allowing the
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

        // Bump instance TTL after successful update.
        bump_instance(&env);

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

    /// Return the total number of streams created through this factory.
    pub fn get_factory_stream_count(env: Env) -> u32 {
        load_stream_ids(&env).len()
    }

    /// Return a page of stream IDs created through this factory.
    ///
    /// `start_index` is a zero-based offset into the full registry. `limit` is
    /// capped at [`MAX_PAGE_SIZE`] (100) to prevent unbounded reads.
    ///
    /// Returns an empty list when `start_index` is beyond the end of the registry.
    pub fn get_factory_streams_paginated(env: Env, start_index: u32, limit: u32) -> Vec<u64> {
        let ids = load_stream_ids(&env);
        let total = ids.len();

        if start_index >= total {
            return vec![&env];
        }

        let capped_limit = limit.min(MAX_PAGE_SIZE);
        let end = (start_index + capped_limit).min(total);
        let mut page = vec![&env];
        for i in start_index..end {
            page.push_back(ids.get(i).unwrap());
        }
        page
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
    /// 6. Rate-per-second bounds check (new)
    /// 7. Cross-contract stream creation
    ///
    /// On success the returned stream ID is appended to the factory's [`DataKey::FactoryStreamIds`]
    /// registry. The registry is only written **after** the cross-contract call succeeds, so a
    /// downstream failure leaves no orphan index entry.
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

        // ── Guard 6: rate bounds ─────────────────────────────────────────────
        // Unset bounds are permissive. Bounds are inclusive.
        if let Some(min_rate) = env.storage().instance().get::<_, i128>(&DataKey::MinRatePerSecond) {
            if rate_per_second < min_rate {
                return Err(FactoryError::RateBelowMin);
            }
        }
        if let Some(max_rate) = env.storage().instance().get::<_, i128>(&DataKey::MaxRatePerSecond) {
            if rate_per_second > max_rate {
                return Err(FactoryError::RateAboveMax);
            }
        }

        // ── Guard 7: memo length ─────────────────────────────────────────────
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

        // --- Interaction ---
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
            &memo,
            &kind,
        ) {
            Ok(Ok(stream_id)) => {
                // --- Effect (post-interaction): record only after a successful creation ---
                // The registry is written only after the cross-contract call succeeds,
                // so a downstream failure leaves no orphan index entry.
                append_stream_id(&env, stream_id);
                env.events().publish(
                    (symbol_short!("fct_strm"),),
                    FactoryStreamCreated {
                        stream_id,
                        sender,
                        recipient,
                        deposit_amount,
                        rate_per_second,
                    },
                );
                Ok(stream_id)
            }
            // Recognized downstream contract error reported in the success frame.
            Ok(Err(_)) => Err(FactoryError::StreamContractError),
            Err(Ok(StreamContractErr::ContractPaused)) => Err(FactoryError::StreamContractPaused),
            Err(Ok(_)) => Err(FactoryError::StreamContractError),
            Err(Err(_)) => Err(FactoryError::StreamContractError),
        }
    }

    /// Create multiple streams in one atomic factory-wrapped transaction.
    ///
    /// # Guard order (checked strictly in sequence)
    /// 1. **CreationPaused** — rejects immediately, before any policy/loop work,
    ///    to avoid leaking allowlist or policy configuration state during an incident.
    /// 2. Iterative validation of each stream: allowlist, cap, times, duration, and batch cap.
    /// 3. Cross-contract batch stream creation.
    pub fn create_streams(
        env: Env,
        sender: Address,
        streams: Vec<fluxora_stream::CreateStreamParams>,
    ) -> Result<Vec<u64>, FactoryError> {
        sender.require_auth();

        // ── Guard 1: pause check (before any policy/loop work) ──────────────
        // Checked first so that no policy configuration or allowlist state is
        // observable when the factory is in emergency-pause mode.
        let paused: bool = env
            .storage()
            .instance()
            .get(&DataKey::CreationPaused)
            .unwrap_or(false);
        if paused {
            return Err(FactoryError::CreationPaused);
        }

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

        // Bump instance TTL on every stream creation attempt.
        // This helps ensure config persists even during periods with many stream operations.
        bump_instance(&env);

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
