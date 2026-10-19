/// Single authoritative record of the Soroban protocol version this contract
/// targets. All budget-guardrail test ceilings are derived from this constant.
///
/// # How to perform a protocol bump
///
/// 1. Update `SOROBAN_PROTOCOL_VERSION` to the new protocol number.
/// 2. Add a new match arm in `protocol_budget_limits` for the new version with
///    the updated ceilings from the Stellar protocol documentation.
/// 3. Run `cargo test -p fluxora_stream` — the build will fail until step 2 is
///    complete because the wildcard arm panics at compile time.
///
/// # Current pin
///
/// Protocol 21 corresponds to `soroban-sdk 21.7.7` (see `Cargo.toml`).
/// Do not advance past the protocol version live on the target Stellar network.
pub const SOROBAN_PROTOCOL_VERSION: u32 = 21;

/// Resource ceilings derived from a specific Soroban protocol version.
///
/// All fields represent **maximum** allowed values for the named operation.
/// Tests assert that measured costs stay at or below these ceilings.
pub struct ProtocolBudgetLimits {
    /// Maximum CPU instructions for a single `withdraw` call (hot path).
    pub single_withdraw_cpu_max: u64,
    /// Maximum memory bytes for a single `withdraw` call (hot path).
    pub single_withdraw_mem_max: u64,
    /// Maximum CPU instructions for `batch_withdraw` over 10 streams.
    pub batch_withdraw_10_cpu_max: u64,
    /// Maximum memory bytes for `batch_withdraw` over 10 streams.
    pub batch_withdraw_10_mem_max: u64,
    /// Maximum CPU instructions for `create_streams` with 5 entries.
    pub create_streams_5_cpu_max: u64,
    /// Maximum memory bytes for `create_streams` with 5 entries.
    pub create_streams_5_mem_max: u64,
}

/// Maps a Soroban protocol version number to its resource ceilings.
///
/// This function is `const` and is evaluated at compile time when used to
/// initialise `LIMITS`. The wildcard arm calls `panic!`, which turns an
/// unknown protocol version into a **compile error** — CI cannot pass until
/// the developer adds the correct limits for the new version.
///
/// # Panics (compile time)
///
/// Panics — and therefore fails the build — when `version` is not a known,
/// supported Soroban protocol number.
pub const fn protocol_budget_limits(version: u32) -> ProtocolBudgetLimits {
    match version {
        21 => ProtocolBudgetLimits {
            // Ceilings measured against soroban-sdk 21.7.7 on protocol 21.
            // Source: empirical guardrail tests in contracts/stream/src/test.rs.
            // Update this block when migrating to a new protocol version.
            single_withdraw_cpu_max: 1_000_000,
            single_withdraw_mem_max: 500_000,
            batch_withdraw_10_cpu_max: 5_000_000,
            batch_withdraw_10_mem_max: 2_000_000,
            create_streams_5_cpu_max: 3_000_000,
            create_streams_5_mem_max: 1_500_000,
        },
        _ => panic!("unknown Soroban protocol version — add a new arm in protocol_limits.rs"),
    }
}

/// Pre-computed resource ceilings for the pinned protocol version.
///
/// Import this constant in budget-guardrail tests:
///
/// ```rust
/// use crate::protocol_limits::LIMITS;
///
/// assert!(cpu <= LIMITS.single_withdraw_cpu_max, "...");
/// ```
///
/// Changing `SOROBAN_PROTOCOL_VERSION` to an unrecognised value causes this
/// constant to fail at compile time, so CI cannot pass with a stale pin.
pub const LIMITS: ProtocolBudgetLimits = protocol_budget_limits(SOROBAN_PROTOCOL_VERSION);
