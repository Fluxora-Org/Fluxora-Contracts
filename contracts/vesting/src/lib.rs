#![no_std]

use soroban_sdk::{contract, contractimpl, contracttype, symbol_short, token, Address, Env};

// ---------------------------------------------------------------------------
// TTL constants (aligned with stream contract)
// ---------------------------------------------------------------------------

const INSTANCE_LIFETIME_THRESHOLD: u32 = 17_280;
const INSTANCE_BUMP_AMOUNT: u32 = 120_960;
const PERSISTENT_LIFETIME_THRESHOLD: u32 = 17_280;
const PERSISTENT_BUMP_AMOUNT: u32 = 120_960;

pub const CONTRACT_VERSION: u32 = 1;

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Config {
    pub token: Address,
    pub admin: Address,
}

/// Status of a vesting schedule.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VestingStatus {
    /// Tokens are accruing; recipient may claim at any time after cliff.
    Active = 0,
    /// All tokens have been claimed; schedule is fully settled.
    Completed = 1,
    /// Admin revoked the schedule; unvested tokens returned to benefactor.
    Revoked = 2,
}

/// Errors returned by the vesting contract.
#[soroban_sdk::contracterror]
#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum ContractError {
    /// Requested vesting schedule does not exist.
    ScheduleNotFound = 1,
    /// Operation is invalid for the current schedule state.
    InvalidState = 2,
    /// One or more parameters are out of range or logically inconsistent.
    InvalidParams = 3,
    /// Contract is already initialised.
    AlreadyInitialised = 4,
    /// Caller is not authorised to perform this operation.
    Unauthorized = 5,
    /// Arithmetic overflow in vesting calculations.
    ArithmeticOverflow = 6,
    /// Deposit amount does not cover the total vestable amount.
    InsufficientDeposit = 7,
    /// Start time is before the current ledger timestamp.
    StartTimeInPast = 8,
}

/// A single vesting schedule.
///
/// # Observable semantics
/// - `vested(t) = 0` when `t < cliff_time`.
/// - `vested(t) = min((min(t, end_time) − start_time) × rate_per_second, total_amount)` when `t >= cliff_time`.
/// - `claimable(t) = vested(t) − claimed_amount`.
/// - After revocation, `vested(t)` is frozen at `revoked_at`; the beneficiary may still
///   claim the frozen vested amount.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VestingSchedule {
    pub schedule_id: u64,
    /// Address that funded this schedule and receives unvested tokens on revocation.
    pub benefactor: Address,
    /// Address that receives vested tokens via `claim`.
    pub beneficiary: Address,
    /// Total tokens locked in this schedule.
    pub total_amount: i128,
    /// Tokens released per second after cliff.
    pub rate_per_second: i128,
    /// Ledger timestamp when vesting begins (accrual epoch start).
    pub start_time: u64,
    /// Ledger timestamp before which no tokens are claimable.
    pub cliff_time: u64,
    /// Ledger timestamp when vesting ends (accrual epoch end).
    pub end_time: u64,
    /// Tokens already claimed by the beneficiary.
    pub claimed_amount: i128,
    pub status: VestingStatus,
    /// Ledger timestamp when the schedule was revoked, if applicable.
    pub revoked_at: Option<u64>,
}

/// Event emitted when a vesting schedule is created.
#[contracttype]
#[derive(Clone, Debug)]
pub struct ScheduleCreated {
    pub schedule_id: u64,
    pub benefactor: Address,
    pub beneficiary: Address,
    pub total_amount: i128,
    pub rate_per_second: i128,
    pub start_time: u64,
    pub cliff_time: u64,
    pub end_time: u64,
}

/// Event emitted when a beneficiary claims vested tokens.
#[contracttype]
#[derive(Clone, Debug)]
pub struct TokensClaimed {
    pub schedule_id: u64,
    pub beneficiary: Address,
    pub amount: i128,
}

/// Event emitted when an admin revokes a vesting schedule.
#[contracttype]
#[derive(Clone, Debug)]
pub struct ScheduleRevoked {
    pub schedule_id: u64,
    pub revoked_at: u64,
    pub refund_amount: i128,
}

/// Event emitted when a completed or fully-settled schedule is closed.
#[contracttype]
#[derive(Clone, Debug)]
pub struct ScheduleClosed {
    pub schedule_id: u64,
}

/// Storage key namespace.
///
/// # Evolution policy
/// Never reorder or remove variants. Always append new variants at the end.
///
/// | Discriminant | Variant | Storage type |
/// |---|---|---|
/// | 0 | `Config` | Instance |
/// | 1 | `NextScheduleId` | Instance |
/// | 2 | `Schedule(u64)` | Persistent |
#[contracttype]
pub enum DataKey {
    Config,
    NextScheduleId,
    Schedule(u64),
}

// ---------------------------------------------------------------------------
// Storage helpers
// ---------------------------------------------------------------------------

fn bump_instance(env: &Env) {
    env.storage()
        .instance()
        .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
}

fn get_config(env: &Env) -> Result<Config, ContractError> {
    bump_instance(env);
    env.storage()
        .instance()
        .get(&DataKey::Config)
        .ok_or(ContractError::InvalidState)
}

fn read_next_id(env: &Env) -> u64 {
    bump_instance(env);
    env.storage()
        .instance()
        .get(&DataKey::NextScheduleId)
        .unwrap_or(0u64)
}

fn set_next_id(env: &Env, id: u64) {
    env.storage().instance().set(&DataKey::NextScheduleId, &id);
    bump_instance(env);
}

fn load_schedule(env: &Env, id: u64) -> Result<VestingSchedule, ContractError> {
    let key = DataKey::Schedule(id);
    let s: VestingSchedule = env
        .storage()
        .persistent()
        .get(&key)
        .ok_or(ContractError::ScheduleNotFound)?;
    env.storage()
        .persistent()
        .extend_ttl(&key, PERSISTENT_LIFETIME_THRESHOLD, PERSISTENT_BUMP_AMOUNT);
    Ok(s)
}

fn save_schedule(env: &Env, s: &VestingSchedule) {
    let key = DataKey::Schedule(s.schedule_id);
    env.storage().persistent().set(&key, s);
    env.storage()
        .persistent()
        .extend_ttl(&key, PERSISTENT_LIFETIME_THRESHOLD, PERSISTENT_BUMP_AMOUNT);
}

fn remove_schedule(env: &Env, id: u64) {
    env.storage().persistent().remove(&DataKey::Schedule(id));
}

// ---------------------------------------------------------------------------
// Pure vesting math
// ---------------------------------------------------------------------------

/// Compute vested amount at `current_time`.
///
/// Observable semantics:
/// - Returns `0` before `cliff_time`.
/// - Returns `min((min(t, end_time) − start_time) × rate, total_amount)` after cliff.
/// - Multiplication overflow falls back to `total_amount` (safe upper bound).
pub fn calculate_vested(
    start_time: u64,
    cliff_time: u64,
    end_time: u64,
    rate_per_second: i128,
    total_amount: i128,
    current_time: u64,
) -> i128 {
    if current_time < cliff_time {
        return 0;
    }
    if rate_per_second <= 0 || total_amount <= 0 {
        return 0;
    }
    let effective_time = current_time.min(end_time);
    if effective_time <= start_time {
        return 0;
    }
    let elapsed = (effective_time - start_time) as i128;
    let vested = rate_per_second
        .checked_mul(elapsed)
        .unwrap_or(total_amount);
    vested.min(total_amount).max(0)
}

// ---------------------------------------------------------------------------
// Token helpers
// ---------------------------------------------------------------------------

fn pull_token(env: &Env, from: &Address, amount: i128) -> Result<(), ContractError> {
    let cfg = get_config(env)?;
    token::Client::new(env, &cfg.token).transfer(from, &env.current_contract_address(), &amount);
    Ok(())
}

fn push_token(env: &Env, to: &Address, amount: i128) -> Result<(), ContractError> {
    let cfg = get_config(env)?;
    token::Client::new(env, &cfg.token).transfer(&env.current_contract_address(), to, &amount);
    Ok(())
}

// ---------------------------------------------------------------------------
// Contract
// ---------------------------------------------------------------------------

#[contract]
pub struct FluxoraVesting;

#[contractimpl]
impl FluxoraVesting {
    /// Initialise the contract with the vesting token and admin address.
    ///
    /// Must be called exactly once before any other operation.
    ///
    /// # Authorization
    /// Requires `admin` to sign.
    pub fn init(env: Env, token: Address, admin: Address) -> Result<(), ContractError> {
        admin.require_auth();
        if env.storage().instance().has(&DataKey::Config) {
            return Err(ContractError::AlreadyInitialised);
        }
        env.storage()
            .instance()
            .set(&DataKey::Config, &Config { token, admin });
        env.storage()
            .instance()
            .set(&DataKey::NextScheduleId, &0u64);
        bump_instance(&env);
        Ok(())
    }

    /// Create a new vesting schedule and pull `total_amount` tokens from `benefactor`.
    ///
    /// # Parameters
    /// - `benefactor`: Funder; receives unvested tokens if the schedule is revoked.
    /// - `beneficiary`: Recipient of vested tokens via `claim`.
    /// - `total_amount`: Total tokens to vest (must equal `rate_per_second × (end_time − start_time)`).
    /// - `rate_per_second`: Tokens released per second after cliff.
    /// - `start_time`: Vesting epoch start (must be >= current ledger timestamp).
    /// - `cliff_time`: No tokens claimable before this time (`start_time <= cliff_time <= end_time`).
    /// - `end_time`: Vesting epoch end.
    ///
    /// # Returns
    /// The new `schedule_id`.
    ///
    /// # Authorization
    /// Requires `benefactor` to sign.
    pub fn create_vesting(
        env: Env,
        benefactor: Address,
        beneficiary: Address,
        total_amount: i128,
        rate_per_second: i128,
        start_time: u64,
        cliff_time: u64,
        end_time: u64,
    ) -> Result<u64, ContractError> {
        benefactor.require_auth();

        // Validate params
        if total_amount <= 0 || rate_per_second <= 0 {
            return Err(ContractError::InvalidParams);
        }
        if benefactor == beneficiary {
            return Err(ContractError::InvalidParams);
        }
        let now = env.ledger().timestamp();
        if start_time < now {
            return Err(ContractError::StartTimeInPast);
        }
        if start_time >= end_time {
            return Err(ContractError::InvalidParams);
        }
        if cliff_time < start_time || cliff_time > end_time {
            return Err(ContractError::InvalidParams);
        }
        let duration = (end_time - start_time) as i128;
        let required = rate_per_second
            .checked_mul(duration)
            .ok_or(ContractError::ArithmeticOverflow)?;
        if total_amount < required {
            return Err(ContractError::InsufficientDeposit);
        }

        let schedule_id = read_next_id(&env);
        set_next_id(&env, schedule_id + 1);

        let schedule = VestingSchedule {
            schedule_id,
            benefactor: benefactor.clone(),
            beneficiary: beneficiary.clone(),
            total_amount,
            rate_per_second,
            start_time,
            cliff_time,
            end_time,
            claimed_amount: 0,
            status: VestingStatus::Active,
            revoked_at: None,
        };

        // CEI: persist state before token transfer
        save_schedule(&env, &schedule);
        pull_token(&env, &benefactor, total_amount)?;

        env.events().publish(
            (symbol_short!("created"), schedule_id),
            ScheduleCreated {
                schedule_id,
                benefactor,
                beneficiary,
                total_amount,
                rate_per_second,
                start_time,
                cliff_time,
                end_time,
            },
        );

        Ok(schedule_id)
    }

    /// Claim all currently vested-but-unclaimed tokens.
    ///
    /// # Observable semantics
    /// - Claimable = `vested(now) − claimed_amount`.
    /// - If claimable == 0, returns 0 immediately (no state change, no event).
    /// - When all tokens are claimed, status transitions to `Completed`.
    /// - Revoked schedules: claimable is frozen at `revoked_at`; status stays `Revoked`
    ///   until all frozen tokens are claimed, then transitions to `Completed`.
    ///
    /// # Authorization
    /// Requires `beneficiary` to sign.
    pub fn claim(env: Env, schedule_id: u64) -> Result<i128, ContractError> {
        let mut schedule = load_schedule(&env, schedule_id)?;
        schedule.beneficiary.require_auth();

        if schedule.status == VestingStatus::Completed {
            return Err(ContractError::InvalidState);
        }

        let now = env.ledger().timestamp();
        let eval_time = if let Some(rev) = schedule.revoked_at {
            rev.min(now)
        } else {
            now
        };

        let vested = calculate_vested(
            schedule.start_time,
            schedule.cliff_time,
            schedule.end_time,
            schedule.rate_per_second,
            schedule.total_amount,
            eval_time,
        );

        let claimable = vested
            .checked_sub(schedule.claimed_amount)
            .unwrap_or(0)
            .max(0);

        if claimable == 0 {
            return Ok(0);
        }

        schedule.claimed_amount = schedule
            .claimed_amount
            .checked_add(claimable)
            .ok_or(ContractError::ArithmeticOverflow)?;

        // Transition to Completed when all vested tokens are claimed.
        // For revoked schedules, "all" means the frozen vested amount.
        let fully_claimed = if schedule.revoked_at.is_some() {
            schedule.claimed_amount >= vested
        } else {
            schedule.claimed_amount >= schedule.total_amount
        };

        if fully_claimed && schedule.status != VestingStatus::Revoked {
            schedule.status = VestingStatus::Completed;
        } else if fully_claimed && schedule.status == VestingStatus::Revoked {
            schedule.status = VestingStatus::Completed;
        }

        // CEI: persist before transfer
        save_schedule(&env, &schedule);
        push_token(&env, &schedule.beneficiary, claimable)?;

        env.events().publish(
            (symbol_short!("claimed"), schedule_id),
            TokensClaimed {
                schedule_id,
                beneficiary: schedule.beneficiary.clone(),
                amount: claimable,
            },
        );

        Ok(claimable)
    }

    /// Revoke an active vesting schedule (admin only).
    ///
    /// Freezes accrual at the current ledger timestamp and refunds unvested tokens
    /// to the benefactor. The beneficiary may still claim the frozen vested amount.
    ///
    /// # Authorization
    /// Requires admin to sign.
    pub fn revoke(env: Env, schedule_id: u64) -> Result<(), ContractError> {
        let cfg = get_config(&env)?;
        cfg.admin.require_auth();

        let mut schedule = load_schedule(&env, schedule_id)?;

        if schedule.status != VestingStatus::Active {
            return Err(ContractError::InvalidState);
        }

        let now = env.ledger().timestamp();
        let vested_at_revoke = calculate_vested(
            schedule.start_time,
            schedule.cliff_time,
            schedule.end_time,
            schedule.rate_per_second,
            schedule.total_amount,
            now,
        );

        let refund = schedule
            .total_amount
            .checked_sub(vested_at_revoke)
            .unwrap_or(0)
            .max(0);

        // CEI: persist terminal state before transfer
        schedule.status = VestingStatus::Revoked;
        schedule.revoked_at = Some(now);
        save_schedule(&env, &schedule);

        if refund > 0 {
            push_token(&env, &schedule.benefactor, refund)?;
        }

        env.events().publish(
            (symbol_short!("revoked"), schedule_id),
            ScheduleRevoked {
                schedule_id,
                revoked_at: now,
                refund_amount: refund,
            },
        );

        Ok(())
    }

    /// Remove a fully-settled schedule from storage (permissionless cleanup).
    ///
    /// Only `Completed` schedules (all vested tokens claimed) may be closed.
    /// Revoked schedules with unclaimed frozen tokens cannot be closed.
    pub fn close_schedule(env: Env, schedule_id: u64) -> Result<(), ContractError> {
        let schedule = load_schedule(&env, schedule_id)?;

        if schedule.status != VestingStatus::Completed {
            return Err(ContractError::InvalidState);
        }

        env.events().publish(
            (symbol_short!("closed"), schedule_id),
            ScheduleClosed { schedule_id },
        );

        remove_schedule(&env, schedule_id);
        Ok(())
    }

    // ---------------------------------------------------------------------------
    // View entry-points
    // ---------------------------------------------------------------------------

    /// Return the full vesting schedule struct.
    pub fn get_schedule(env: Env, schedule_id: u64) -> Result<VestingSchedule, ContractError> {
        load_schedule(&env, schedule_id)
    }

    /// Return the claimable amount at the current ledger timestamp.
    pub fn get_claimable(env: Env, schedule_id: u64) -> Result<i128, ContractError> {
        let s = load_schedule(&env, schedule_id)?;
        if s.status == VestingStatus::Completed {
            return Ok(0);
        }
        let now = env.ledger().timestamp();
        let eval_time = if let Some(rev) = s.revoked_at {
            rev.min(now)
        } else {
            now
        };
        let vested = calculate_vested(
            s.start_time,
            s.cliff_time,
            s.end_time,
            s.rate_per_second,
            s.total_amount,
            eval_time,
        );
        Ok(vested.checked_sub(s.claimed_amount).unwrap_or(0).max(0))
    }

    /// Return the vested amount at an arbitrary timestamp (simulation).
    pub fn get_vested_at(env: Env, schedule_id: u64, timestamp: u64) -> Result<i128, ContractError> {
        let s = load_schedule(&env, schedule_id)?;
        Ok(calculate_vested(
            s.start_time,
            s.cliff_time,
            s.end_time,
            s.rate_per_second,
            s.total_amount,
            timestamp,
        ))
    }

    /// Return the token address and admin address.
    pub fn get_config(env: Env) -> Result<Config, ContractError> {
        get_config(&env)
    }

    /// Return the total number of schedules created (monotonic counter).
    pub fn get_schedule_count(env: Env) -> u64 {
        read_next_id(&env)
    }

    /// Return the compile-time contract version.
    pub fn version(_env: Env) -> u32 {
        CONTRACT_VERSION
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod test;
