use soroban_sdk::{token, Address, Env};

use super::ContractError;

/// Smoke-test a candidate token contract for SEP-41 compatibility.
///
/// This helper is called during initialization before the selected token address
/// is stored in contract configuration. It exercises two on-chain operations
/// that must exist on a compliant token:
/// - `balance(contract_address)`
/// - `transfer(contract_address, contract_address, 0)`
///
/// The zero-amount self-transfer is a no-op on compliant tokens and verifies that
/// the token contract exposes the expected `transfer` entry-point without
/// requiring an actual balance or allowance.
pub fn verify_token_behavior(env: &Env, token_address: &Address) -> Result<(), ContractError> {
    let token_client = token::Client::new(env, token_address);

    token_client.balance(&env.current_contract_address());
    token_client.transfer(
        &env.current_contract_address(),
        &env.current_contract_address(),
        &0_i128,
    );

    Ok(())
}
