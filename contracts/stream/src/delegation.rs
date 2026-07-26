//! Delegation parameter validation for delegated-withdraw operations.

use soroban_sdk::Env;

use crate::{load_delegated_nonce, load_stream, ContractError};

/// Domain-separation tag for witnessed cancellation signatures.
///
/// Prepended to the signed payload so a witness attestation cannot be replayed
/// as a `delegated_withdraw` signature (which uses a distinct byte layout).
pub(crate) const WITNESSED_CANCEL_DOMAIN: &[u8; 24] = b"fluxora_witnessed_cancel";

/// Validate the deadline for a witnessed cancellation attestation.
///
/// Checks `deadline >= env.ledger().timestamp()` — rejects expired signatures
/// before any stream state is read.
pub(crate) fn validate_witness_cancel_deadline(
    env: &Env,
    deadline: u64,
) -> Result<(), ContractError> {
    if env.ledger().timestamp() > deadline {
        return Err(ContractError::SignatureDeadlineExpired);
    }
    Ok(())
}

/// Build the signed message for witnessed cancellation.
///
/// Layout: `WITNESSED_CANCEL_DOMAIN` | `stream_id` (8 bytes, big-endian u64)
/// | `deadline` (8 bytes, big-endian u64).
pub(crate) fn build_witnessed_cancel_message(
    env: &Env,
    stream_id: u64,
    deadline: u64,
) -> soroban_sdk::Bytes {
    let mut msg = soroban_sdk::Bytes::new(env);
    msg.extend_from_slice(WITNESSED_CANCEL_DOMAIN);
    msg.extend_from_array(&stream_id.to_be_bytes());
    msg.extend_from_array(&deadline.to_be_bytes());
    msg
}

/// Validate the delegation parameters for a delegated-withdraw call.
///
/// Checks, in order:
/// 1. `relayer_fee >= 0` — rejects negative fee parameters.
/// 2. `deadline >= env.ledger().timestamp()` — rejects expired signatures.
/// 3. `nonce == current_nonce(stream.recipient)` — rejects replays.
pub(crate) fn validate_delegation_params(
    env: &Env,
    stream_id: u64,
    nonce: u64,
    deadline: u64,
    relayer_fee: i128,
) -> Result<(), ContractError> {
    if relayer_fee < 0 {
        return Err(ContractError::InvalidParams);
    }

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
    use crate::{CreateStreamParams, FluxoraStream, FluxoraStreamClient, StreamKind};
    use soroban_sdk::{
        testutils::{Address as _, Ledger},
        token::Client as TokenClient,
        Address, Env,
    };

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

        let sac = soroban_sdk::token::StellarAssetClient::new(&env, &token_id);
        sac.mint(&sender, &10_000_i128);
        TokenClient::new(&env, &token_id).approve(&sender, &contract_id, &i128::MAX, &100_000);

        env.ledger().set_timestamp(0);
        let stream_id = client.create_stream(
            &sender,
            &CreateStreamParams {
                recipient: recipient.clone(),
                deposit_amount: 1000_i128,
                rate_per_second: 1_i128,
                start_time: 0u64,
                cliff_time: 0u64,
                end_time: 1000u64,
                withdraw_dust_threshold: Some(0),
                memo: None,
                metadata: None,
                kind: StreamKind::Linear,
                irrevocable: None,
                witness: None,
            },
        );

        (env, client, stream_id, recipient)
    }

    #[test]
    fn test_valid_relayer_fee_passes() {
        let (env, client, stream_id, _recipient) = setup();
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
            &CreateStreamParams {
                recipient: recipient_a.clone(),
                deposit_amount: 1000_i128,
                rate_per_second: 1_i128,
                start_time: 0u64,
                cliff_time: 0u64,
                end_time: 1000u64,
                withdraw_dust_threshold: Some(0),
                memo: None,
                metadata: None,
                kind: StreamKind::Linear,
                irrevocable: None,
                witness: None,
            },
        );
        let _stream_b = client.create_stream(
            &sender,
            &CreateStreamParams {
                recipient: recipient_b.clone(),
                deposit_amount: 1000_i128,
                rate_per_second: 1_i128,
                start_time: 0u64,
                cliff_time: 0u64,
                end_time: 1000u64,
                withdraw_dust_threshold: Some(0),
                memo: None,
                metadata: None,
                kind: StreamKind::Linear,
                irrevocable: None,
                witness: None,
            },
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

    // ── Witnessed cancel deadline ───────────────────────────────────────

    /// Deadline exactly equal to the current timestamp must pass.
    #[test]
    fn test_witness_cancel_deadline_equal_to_now_passes() {
        let (env, _client, _stream_id, _recipient) = setup();
        env.ledger().set_timestamp(100);

        let result = env.as_contract(&_client.address, || {
            validate_witness_cancel_deadline(&env, 100)
        });
        assert_eq!(result, Ok(()));
    }

    /// Deadline one second before the current timestamp must fail.
    #[test]
    fn test_witness_cancel_deadline_expired_fails() {
        let (env, _client, _stream_id, _recipient) = setup();
        env.ledger().set_timestamp(101);

        let result = env.as_contract(&_client.address, || {
            validate_witness_cancel_deadline(&env, 100)
        });
        assert_eq!(result, Err(ContractError::SignatureDeadlineExpired));
    }

    /// Witness cancel message includes domain separation tag.
    #[test]
    fn test_witness_cancel_message_domain_separated() {
        let (env, _client, stream_id, _recipient) = setup();
        let msg = build_witnessed_cancel_message(&env, stream_id, 500);
        assert!(msg.len() >= WITNESSED_CANCEL_DOMAIN.len() as u32 + 16);
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
            validate_delegation_params(&env, stream_id, 0, 100, 10)
        });
        assert_eq!(result, Ok(()));
    }

    #[test]
    fn test_negative_relayer_fee_fails() {
        let (env, client, stream_id, _recipient) = setup();
        env.ledger().set_timestamp(100);

        let result = env.as_contract(&client.address, || {
            validate_delegation_params(&env, stream_id, 0, 100, -1)
        });
        assert_eq!(result, Err(ContractError::InvalidParams));
    }
}
