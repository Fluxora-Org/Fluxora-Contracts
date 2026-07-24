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

use crate::{load_delegated_nonce, load_stream, ContractError};

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
/// - `Err(ContractError::InvalidSignature)` if `nonce` does not match.
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
    let current_nonce = load_delegated_nonce(env, &stream.recipient);
    if nonce != current_nonce {
        return Err(ContractError::InvalidSignature);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use crate::{FluxoraStream, FluxoraStreamClient, StreamKind};
    use soroban_sdk::{
        testutils::{Address as _, Ledger},
        token::Client as TokenClient,
        Address, Env,
    };

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

        let result = env.as_contract(&_client.address, || {
            validate_delegation_params(&env, stream_id, 0, 100)
        });
        assert_eq!(result, Ok(()));
    }

    /// Deadline one second before the current timestamp must fail.
    #[test]
    fn test_deadline_one_second_before_fails() {
        let (env, _client, stream_id, _recipient) = setup();
        env.ledger().set_timestamp(101);

        let result = env.as_contract(&_client.address, || {
            validate_delegation_params(&env, stream_id, 0, 100)
        });
        assert_eq!(result, Err(ContractError::SignatureDeadlineExpired));
    }

    /// Nonce equal to the stored nonce (0) must pass.
    #[test]
    fn test_nonce_equal_passes() {
        let (env, _client, stream_id, _recipient) = setup();
        env.ledger().set_timestamp(50);

        let result = env.as_contract(&_client.address, || {
            validate_delegation_params(&env, stream_id, 0, 100)
        });
        assert_eq!(result, Ok(()));
    }

    /// Nonce off-by-one (1 when stored is 0) must fail with InvalidSignature.
    #[test]
    fn test_nonce_off_by_one_fails() {
        let (env, _client, stream_id, _recipient) = setup();
        env.ledger().set_timestamp(50);

        let result = env.as_contract(&_client.address, || {
            validate_delegation_params(&env, stream_id, 1, 100)
        });
        assert_eq!(result, Err(ContractError::InvalidSignature));
    }

    /// Nonexistent stream_id must fail with StreamNotFound.
    #[test]
    fn test_missing_stream_fails() {
        let (env, _client, _stream_id, _recipient) = setup();
        env.ledger().set_timestamp(50);

        let result = env.as_contract(&_client.address, || {
            validate_delegation_params(&env, 999, 0, 100)
        });
        assert_eq!(result, Err(ContractError::StreamNotFound));
    }

    // ── Deadline boundary ───────────────────────────────────────────────

    /// deadline=0 when timestamp > 0 must fail.
    #[test]
    fn test_deadline_zero_fails() {
        let (env, _client, stream_id, _recipient) = setup();
        env.ledger().set_timestamp(1);

        let result = env.as_contract(&_client.address, || {
            validate_delegation_params(&env, stream_id, 0, 0)
        });
        assert_eq!(result, Err(ContractError::SignatureDeadlineExpired));
    }

    /// deadline=u64::MAX must pass (well into the future).
    #[test]
    fn test_deadline_max_value_passes() {
        let (env, _client, stream_id, _recipient) = setup();
        env.ledger().set_timestamp(100);

        let result = env.as_contract(&_client.address, || {
            validate_delegation_params(&env, stream_id, 0, u64::MAX)
        });
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
        let result = env.as_contract(&_client.address, || {
            validate_delegation_params(&env, stream_id, 999, 100)
        });
        assert_eq!(result, Err(ContractError::SignatureDeadlineExpired));
    }

    /// When stream doesn't exist AND nonce is wrong, StreamNotFound wins
    /// because stream lookup happens before nonce check.
    #[test]
    fn test_stream_not_found_checked_before_nonce() {
        let (env, _client, _stream_id, _recipient) = setup();
        env.ledger().set_timestamp(50);

        // stream_id=999 doesn't exist, nonce=999 is wrong — StreamNotFound first
        let result = env.as_contract(&_client.address, || {
            validate_delegation_params(&env, 999, 999, 100)
        });
        assert_eq!(result, Err(ContractError::StreamNotFound));
    }

    // ── Nonce boundary ──────────────────────────────────────────────────

    /// nonce=u64::MAX when stored nonce is 0 must fail.
    #[test]
    fn test_nonce_max_value_fails() {
        let (env, _client, stream_id, _recipient) = setup();
        env.ledger().set_timestamp(50);

        let result = env.as_contract(&_client.address, || {
            validate_delegation_params(&env, stream_id, u64::MAX, 100)
        });
        assert_eq!(result, Err(ContractError::InvalidSignature));
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
            env.as_contract(&contract_id, || validate_delegation_params(
                &env, stream_a, 0, 100
            )),
            Ok(())
        );
        assert_eq!(
            env.as_contract(&contract_id, || validate_delegation_params(
                &env, _stream_b, 0, 100
            )),
            Ok(())
        );

        // Nonce 1 must fail for both (stored is 0)
        assert_eq!(
            env.as_contract(&contract_id, || validate_delegation_params(
                &env, stream_a, 1, 100
            )),
            Err(ContractError::InvalidSignature)
        );
        assert_eq!(
            env.as_contract(&contract_id, || validate_delegation_params(
                &env, _stream_b, 1, 100
            )),
            Err(ContractError::InvalidSignature)
        );
    }

    /// A failed validate_delegation_params call must not consume the nonce;
    /// a subsequent call with the correct nonce must still succeed.
    #[test]
    fn test_failed_validation_does_not_consume_nonce() {
        let (env, _client, stream_id, _recipient) = setup();
        env.ledger().set_timestamp(50);

        // First call: wrong nonce → fails
        let result_fail = env.as_contract(&_client.address, || {
            validate_delegation_params(&env, stream_id, 1, 100)
        });
        assert_eq!(result_fail, Err(ContractError::InvalidSignature));

        // Second call: correct nonce → must still succeed
        let result_ok = env.as_contract(&_client.address, || {
            validate_delegation_params(&env, stream_id, 0, 100)
        });
        assert_eq!(result_ok, Ok(()));
    }

    // ── Delegation Revocation & Live State Tests ─────────────────────────

    /// Legitimate pre-revocation case: delegation granted and used before any revocation
    /// must succeed.
    #[test]
    fn test_pre_revocation_delegated_withdraw_succeeds() {
        let (env, client, stream_id, recipient) = setup();
        env.ledger().set_timestamp(50);

        // Delegation granted with stored nonce (0) before any revocation.
        let result = env.as_contract(&client.address, || {
            validate_delegation_params(&env, stream_id, 0, 100)
        });
        assert_eq!(result, Ok(()));

        // Nonce remains 0 in storage
        let stored = env.as_contract(&client.address, || load_delegated_nonce(&env, &recipient));
        assert_eq!(stored, 0);
    }

    /// Same-ledger revocation race: revoking delegation immediately blocks any
    /// in-flight or subsequent delegated withdraw attempt processed at or after
    /// the revoking transaction within the same ledger.
    #[test]
    fn test_same_ledger_revocation_race_blocks_withdraw() {
        let (env, client, stream_id, recipient) = setup();
        env.ledger().set_timestamp(50);

        // 1. Prior to revocation, nonce 0 is valid.
        let result_before = env.as_contract(&client.address, || {
            validate_delegation_params(&env, stream_id, 0, 100)
        });
        assert_eq!(result_before, Ok(()));

        // 2. Revoke delegation in the same ledger (advance stored nonce to 1).
        env.as_contract(&client.address, || {
            crate::storage::increment_delegated_nonce(&env, &recipient);
        });

        // 3. Attempt withdraw using revoked delegation (nonce 0) in the same ledger (timestamp 50).
        // Must be strictly rejected with no window where the just-revoked delegate can withdraw.
        let result_after = env.as_contract(&client.address, || {
            validate_delegation_params(&env, stream_id, 0, 100)
        });
        assert_eq!(result_after, Err(ContractError::InvalidSignature));
    }

    /// Later-ledger revocation: revoking delegation blocks withdraw attempts in later ledgers.
    #[test]
    fn test_later_ledger_revocation_blocks_withdraw() {
        let (env, client, stream_id, recipient) = setup();
        env.ledger().set_timestamp(50);

        // Revoke delegation at t=50 (advance nonce).
        env.as_contract(&client.address, || {
            crate::storage::increment_delegated_nonce(&env, &recipient);
        });

        // Advance to a later ledger timestamp.
        env.ledger().set_timestamp(150);

        // Attempt withdraw using old/revoked delegation (nonce 0) in later ledger → rejected.
        let result = env.as_contract(&client.address, || {
            validate_delegation_params(&env, stream_id, 0, 200)
        });
        assert_eq!(result, Err(ContractError::InvalidSignature));
    }

    /// Authorization check live state verification: confirms that the authorization check
    /// reads delegation state fresh on every call rather than from a cached/stale value.
    #[test]
    fn test_authorization_reads_live_delegation_state_fresh() {
        let (env, client, stream_id, recipient) = setup();
        env.ledger().set_timestamp(50);

        // First authorization check with stored nonce 0 → succeeds
        let check_1 = env.as_contract(&client.address, || {
            validate_delegation_params(&env, stream_id, 0, 100)
        });
        assert_eq!(check_1, Ok(()));

        // Storage state is modified in the contract env (nonce incremented to 1)
        env.as_contract(&client.address, || {
            crate::storage::increment_delegated_nonce(&env, &recipient);
        });

        // Second authorization check with nonce 0 without modifying parameters → fails immediately
        let check_2 = env.as_contract(&client.address, || {
            validate_delegation_params(&env, stream_id, 0, 100)
        });
        assert_eq!(check_2, Err(ContractError::InvalidSignature));

        // Authorization check with updated nonce 1 → succeeds
        let check_3 = env.as_contract(&client.address, || {
            validate_delegation_params(&env, stream_id, 1, 100)
        });
        assert_eq!(check_3, Ok(()));
    }
}
