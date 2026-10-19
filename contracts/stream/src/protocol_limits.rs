/// Calibration version for the budget-guardrail ceilings below.
///
/// This records the legacy protocol-21 measurements, not the workspace's
/// current SDK or the protocol running on the target network.
///
/// # How to perform a protocol bump
///
/// 1. Add a new match arm in `protocol_budget_limits` with calibrated ceilings.
/// 2. Update `SOROBAN_PROTOCOL_VERSION` to that protocol number.
/// 3. Run `cargo test -p fluxora-stream`. An unsupported pin fails at compile
///    time when `LIMITS` is evaluated; the tests also check the pin's mapping.
pub const SOROBAN_PROTOCOL_VERSION: u32 = 21;

/// Resource ceilings derived from a specific Soroban protocol version.
///
/// All fields represent **maximum** allowed values for the named operation.
/// Tests assert that measured costs stay at or below these ceilings.
#[derive(Debug, PartialEq, Eq)]
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

/// No calibrated budget ceilings exist for the requested protocol.
#[derive(Debug, PartialEq, Eq)]
pub struct UnsupportedProtocolVersion {
    pub requested_version: u32,
}

impl core::fmt::Display for UnsupportedProtocolVersion {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(
            f,
            "Soroban protocol version {} has no calibrated resource ceilings; add a match arm in protocol_budget_limits",
            self.requested_version
        )
    }
}

/// Maps a Soroban protocol version number to its calibrated resource ceilings.
///
/// Unknown versions return an error carrying the requested version rather than
/// panicking or silently falling back to another protocol's ceilings. Returning
/// an error also keeps the lookup const-evaluable without formatted const panics.
pub const fn protocol_budget_limits(
    version: u32,
) -> Result<ProtocolBudgetLimits, UnsupportedProtocolVersion> {
    match version {
        21 => Ok(ProtocolBudgetLimits {
            // Ceilings measured against soroban-sdk 21.7.7 on protocol 21.
            // Source: empirical guardrail tests in contracts/stream/src/test.rs.
            // Preserve these values when adding a new protocol's calibration.
            single_withdraw_cpu_max: 1_000_000,
            single_withdraw_mem_max: 500_000,
            batch_withdraw_10_cpu_max: 5_000_000,
            batch_withdraw_10_mem_max: 2_000_000,
            create_streams_5_cpu_max: 3_000_000,
            create_streams_5_mem_max: 1_500_000,
        }),
        _ => Err(UnsupportedProtocolVersion {
            requested_version: version,
        }),
    }
}

/// Pre-computed resource ceilings for the pinned calibration version.
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
pub const LIMITS: ProtocolBudgetLimits = match protocol_budget_limits(SOROBAN_PROTOCOL_VERSION) {
    Ok(limits) => limits,
    Err(_) => panic!(
        "SOROBAN_PROTOCOL_VERSION has no calibrated resource ceilings; add a protocol_budget_limits match arm before changing the pin"
    ),
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_21_ceilings_are_unchanged() {
        assert_eq!(
            protocol_budget_limits(21),
            Ok(ProtocolBudgetLimits {
                single_withdraw_cpu_max: 1_000_000,
                single_withdraw_mem_max: 500_000,
                batch_withdraw_10_cpu_max: 5_000_000,
                batch_withdraw_10_mem_max: 2_000_000,
                create_streams_5_cpu_max: 3_000_000,
                create_streams_5_mem_max: 1_500_000,
            })
        );
    }

    #[test]
    fn unsupported_protocols_report_the_requested_version() {
        for requested_version in [0, 20, 22, 27, u32::MAX] {
            assert_eq!(
                protocol_budget_limits(requested_version),
                Err(UnsupportedProtocolVersion { requested_version })
            );
        }
    }

    #[test]
    fn unsupported_protocol_message_names_the_version_and_remedy() {
        let error = protocol_budget_limits(27).expect_err("protocol 27 is not calibrated");
        assert_eq!(
            std::format!("{error}"),
            "Soroban protocol version 27 has no calibrated resource ceilings; add a match arm in protocol_budget_limits"
        );
    }

    #[test]
    fn pinned_protocol_has_matching_calibrated_limits() {
        let limits = protocol_budget_limits(SOROBAN_PROTOCOL_VERSION)
            .expect("SOROBAN_PROTOCOL_VERSION must have a calibrated match arm");
        assert_eq!(LIMITS, limits);
    }
}
