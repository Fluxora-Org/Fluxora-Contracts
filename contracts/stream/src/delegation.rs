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

use crate::{load_stream, load_delegated_nonce, ContractError};

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
#[allow(dead_code)]
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
    let current_nonce = load_delegated_nonce(env, &stream.recipient);
    if nonce != current_nonce {
        return Err(ContractError::InvalidParams);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use soroban_sdk::{
        testutils::{Address as _, Ledger},
        token::Client as TokenClient,
        Address, Env,
    };
    use crate::{FluxoraStream, FluxoraStreamClient, StreamKind};

    /// Set up a minimal contract environment and return (env, client, stream_id, recipient).
    fn setup() -> (Env, FluxoraStreamClient<'static>, u64, Address) {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register_contract(None, FluxoraStream);
        let token_admin = Address::generate(&env);
        let token_id = env
            .register_stellar_asset_contract_v2(token_admin.clone())
            .address();
        let admin = Address::generate(&env);
        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);

        let client = FluxoraStreamClient::new(&env, &contract_id);
        client.init(&token_id, &admin);

        // Mint tokens to sender and approve the contract
        let sac = soroban_sdk::token::StellarAssetClient::new(&env, &token_id);
        sac.mint(&sender, &10_000_i128);
        TokenClient::new(&env, &token_id).approve(&sender, &contract_id, &i128::MAX, &100_000);

        // Create a default stream (deposit=1000, rate=1/s, 0..1000s, no cliff)
        env.ledger().set_timestamp(0);
        let stream_id = client.create_stream(
            &sender,
            &recipient,
            &1000_i128,
            &1_i128,
            &0u64,
            &0u64,
            &1000u64,
            &0,
            &None,
            &StreamKind::Linear,
        );

        (env, client, stream_id, recipient)
    }

    /// Deadline exactly equal to the current timestamp must pass.
    #[test]
    fn test_deadline_equal_to_now_passes() {
        let (env, _client, stream_id, _recipient) = setup();
        env.ledger().set_timestamp(100);

        let result = validate_delegation_params(&env, stream_id, 0, 100);
        assert_eq!(result, Ok(()));
    }

    /// Deadline one second before the current timestamp must fail.
    #[test]
    fn test_deadline_one_second_before_fails() {
        let (env, _client, stream_id, _recipient) = setup();
        env.ledger().set_timestamp(101);

        let result = validate_delegation_params(&env, stream_id, 0, 100);
        assert_eq!(result, Err(ContractError::SignatureDeadlineExpired));
    }

    /// Nonce equal to the stored nonce (0) must pass.
    #[test]
    fn test_nonce_equal_passes() {
        let (env, _client, stream_id, _recipient) = setup();
        env.ledger().set_timestamp(50);

        let result = validate_delegation_params(&env, stream_id, 0, 100);
        assert_eq!(result, Ok(()));
    }

    /// Nonce off-by-one (1 when stored is 0) must fail with InvalidParams.
    #[test]
    fn test_nonce_off_by_one_fails() {
        let (env, _client, stream_id, _recipient) = setup();
        env.ledger().set_timestamp(50);

        let result = validate_delegation_params(&env, stream_id, 1, 100);
        assert_eq!(result, Err(ContractError::InvalidParams));
    }

    /// Nonexistent stream_id must fail with StreamNotFound.
    #[test]
    fn test_missing_stream_fails() {
        let (env, _client, _stream_id, _recipient) = setup();
        env.ledger().set_timestamp(50);

        let result = validate_delegation_params(&env, 999, 0, 100);
        assert_eq!(result, Err(ContractError::StreamNotFound));
    }

    // ── Deadline boundary ───────────────────────────────────────────────

    /// deadline=0 when timestamp > 0 must fail.
    #[test]
    fn test_deadline_zero_fails() {
        let (env, _client, stream_id, _recipient) = setup();
        env.ledger().set_timestamp(1);

        let result = validate_delegation_params(&env, stream_id, 0, 0);
        assert_eq!(result, Err(ContractError::SignatureDeadlineExpired));
    }

    /// deadline=u64::MAX must pass (well into the future).
    #[test]
    fn test_deadline_max_value_passes() {
        let (env, _client, stream_id, _recipient) = setup();
        env.ledger().set_timestamp(100);

        let result = validate_delegation_params(&env, stream_id, 0, u64::MAX);
        assert_eq!(result, Ok(()));
    }

    // ── Validation ordering ─────────────────────────────────────────────

    /// When both deadline and nonce are wrong, SignatureDeadlineExpired wins
    /// because deadline is checked before nonce.
    #[test]
    fn test_deadline_checked_before_nonce() {
        let (env, _client, stream_id, _recipient) = setup();
        env.ledger().set_timestamp(200);

        // deadline=100 is expired, nonce=999 is wrong — deadline error first
        let result = validate_delegation_params(&env, stream_id, 999, 100);
        assert_eq!(result, Err(ContractError::SignatureDeadlineExpired));
    }

    /// When stream doesn't exist AND nonce is wrong, StreamNotFound wins
    /// because stream lookup happens before nonce check.
    #[test]
    fn test_stream_not_found_checked_before_nonce() {
        let (env, _client, _stream_id, _recipient) = setup();
        env.ledger().set_timestamp(50);

        // stream_id=999 doesn't exist, nonce=999 is wrong — StreamNotFound first
        let result = validate_delegation_params(&env, 999, 999, 100);
        assert_eq!(result, Err(ContractError::StreamNotFound));
    }

    // ── Nonce boundary ──────────────────────────────────────────────────

    /// nonce=u64::MAX when stored nonce is 0 must fail.
    #[test]
    fn test_nonce_max_value_fails() {
        let (env, _client, stream_id, _recipient) = setup();
        env.ledger().set_timestamp(50);

        let result = validate_delegation_params(&env, stream_id, u64::MAX, 100);
        assert_eq!(result, Err(ContractError::InvalidParams));
    }

    // ── Nonce invariants ────────────────────────────────────────────────

    /// Nonce is scoped per-recipient: creating a second stream with a
    /// different recipient must not affect the first recipient's nonce.
    #[test]
    fn test_nonce_is_per_recipient() {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register_contract(None, FluxoraStream);
        let token_admin = Address::generate(&env);
        let token_id = env
            .register_stellar_asset_contract_v2(token_admin.clone())
            .address();
        let admin = Address::generate(&env);
        let sender = Address::generate(&env);
        let recipient_a = Address::generate(&env);
        let recipient_b = Address::generate(&env);

        let client = FluxoraStreamClient::new(&env, &contract_id);
        client.init(&token_id, &admin);

        let sac = soroban_sdk::token::StellarAssetClient::new(&env, &token_id);
        sac.mint(&sender, &10_000_i128);
        TokenClient::new(&env, &token_id).approve(&sender, &contract_id, &i128::MAX, &100_000);

        env.ledger().set_timestamp(0);
        let stream_a = client.create_stream(
            &sender,
            &recipient_a,
            &1000_i128,
            &1_i128,
            &0u64,
            &0u64,
            &1000u64,
            &0,
            &None,
            &StreamKind::Linear,
        );
        let _stream_b = client.create_stream(
            &sender,
            &recipient_b,
            &1000_i128,
            &1_i128,
            &0u64,
            &0u64,
            &1000u64,
            &0,
            &None,
            &StreamKind::Linear,
        );

        env.ledger().set_timestamp(50);

        // Both recipients have default nonce=0; nonce 0 must pass for both
        assert_eq!(
            validate_delegation_params(&env, stream_a, 0, 100),
            Ok(())
        );
        assert_eq!(
            validate_delegation_params(&env, _stream_b, 0, 100),
            Ok(())
        );

        // Nonce 1 must fail for both (stored is 0)
        assert_eq!(
            validate_delegation_params(&env, stream_a, 1, 100),
            Err(ContractError::InvalidParams)
        );
        assert_eq!(
            validate_delegation_params(&env, _stream_b, 1, 100),
            Err(ContractError::InvalidParams)
        );
    }

    /// A failed validate_delegation_params call must not consume the nonce;
    /// a subsequent call with the correct nonce must still succeed.
    #[test]
    fn test_failed_validation_does_not_consume_nonce() {
        let (env, _client, stream_id, _recipient) = setup();
        env.ledger().set_timestamp(50);

        // First call: wrong nonce → fails
        let result_fail = validate_delegation_params(&env, stream_id, 1, 100);
        assert_eq!(result_fail, Err(ContractError::InvalidParams));

        // Second call: correct nonce → must still succeed
        let result_ok = validate_delegation_params(&env, stream_id, 0, 100);
        assert_eq!(result_ok, Ok(()));
    }
}
