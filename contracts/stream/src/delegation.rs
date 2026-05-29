//! Delegation parameter validation for delegated-withdraw operations.
//!
//! This module centralises the deadline and nonce checks that guard
//! [`FluxoraStream::delegated_withdraw`].  Extracting them here ensures:
//!
//! - A single authoritative location for delegation security logic.
//! - Consistent error codes (`SignatureDeadlineExpired`, `InvalidParams`) across
//!   any future delegated operations.
//! - An easy-to-audit surface: auditors can review this file in isolation.
//!
//! # Security invariants
//!
//! 1. **Deadline check** — `deadline` must be `>= env.ledger().timestamp()`.
//!    Expired signatures are rejected before any state is read.
//! 2. **Nonce check** — `nonce` must equal the stored per-recipient nonce exactly.
//!    Any mismatch (replay or out-of-order submission) is rejected.
//!
//! Neither check consumes the nonce; that is the caller's responsibility after
//! all other validation (signature verification, stream status) passes.

use soroban_sdk::Env;

use crate::{load_stream, read_withdraw_nonce, ContractError};

/// Validate the delegation parameters for a delegated-withdraw call.
///
/// Checks, in order:
/// 1. `deadline >= env.ledger().timestamp()` — rejects expired signatures.
/// 2. `nonce == current_nonce(stream.recipient)` — rejects replays.
///
/// # Parameters
/// - `env`: Contract environment (used for ledger timestamp and storage reads).
/// - `stream_id`: Stream being withdrawn from (used to look up the recipient).
/// - `nonce`: Caller-supplied nonce; must match the recipient's stored nonce.
/// - `deadline`: Ledger timestamp after which the signature is invalid.
///
/// # Returns
/// - `Ok(())` if both checks pass.
/// - `Err(ContractError::SignatureDeadlineExpired)` if `deadline < current timestamp`.
/// - `Err(ContractError::InvalidParams)` if `nonce` does not match.
/// - `Err(ContractError::StreamNotFound)` if `stream_id` does not exist.
pub(crate) fn validate_delegation_params(
    env: &Env,
    stream_id: u64,
    nonce: u64,
    deadline: u64,
) -> Result<(), ContractError> {
    if env.ledger().timestamp() > deadline {
        return Err(ContractError::SignatureDeadlineExpired);
    }

    let stream = load_stream(env, stream_id)?;
    let current_nonce = read_withdraw_nonce(env, &stream.recipient);
    if nonce != current_nonce {
        return Err(ContractError::InvalidParams);
    }

    Ok(())
}
