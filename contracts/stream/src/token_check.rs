use soroban_sdk::{token, Address, Env};

use super::ContractError;

/// Smoke-test a candidate token contract for SEP-41 compatibility and balance invariants.
///
/// This helper is called during initialization before the selected token address
/// is stored in contract configuration. It exercises two on-chain operations
/// that must exist on a compliant token:
/// - `balance(contract_address)`
/// - `transfer(contract_address, contract_address, 0)`
///
/// ### Balance Invariants
/// The token's balance at the contract address is queried before and after a zero-value
/// self-transfer. Compliant tokens must not change the balance of any account during
/// a zero-value transfer. Specifically, the initial and final balances must be identical.
///
/// ### Malformed Balance Rejection
/// Asset balances represent physical or digital quantities and must be non-negative.
/// Balance values below zero are mathematically malformed and indicate a corrupt,
/// non-compliant, or adversarial token contract. Any negative balance read returns
/// `ContractError::TokenVerificationFailed`.
///
/// ### Panicking Token Rejection
/// Some SEP-41 implementations choose to panic/revert on zero-value transfers. The
/// zero-value self-transfer is wrapped via `try_transfer` so that a panicking token
/// is caught and converted to `ContractError::TokenRevertedOnZeroTransfer` rather
/// than propagating as an unhandled host panic. This is distinct from
/// `TokenVerificationFailed` which covers tokens that succeed but return invalid
/// balance values or change the balance across the no-op transfer.
///
/// ### Security Rationale
/// Enforcing that zero-value transfers are strict no-ops and that balances are non-negative
/// prevents integration with flawed, bugged, or malicious tokens that could manipulate
/// contract balances, trigger underflows/overflows, or corrupt internal accounting invariants
/// (e.g., total liabilities vs contract holding).
///
/// ### Zero-Balance / Zero-Allowance Compatibility
/// The smoke test uses a zero-value self-transfer. Compliant SEP-41 tokens execute
/// zero-value transfers successfully even if the caller has a zero balance or zero allowance.
/// This ensures the initialization smoke test remains compatible with clean deployments
/// without requiring bootstrapping funds or allowances.
pub fn verify_token_behavior(env: &Env, token_address: &Address) -> Result<(), ContractError> {
    let token_client = token::Client::new(env, token_address);
    let contract_addr = env.current_contract_address();

    // 1. Read the token balance before the zero-value self-transfer.
    let initial_balance = token_client.balance(&contract_addr);

    // Reject negative balances as they are mathematically malformed.
    if initial_balance < 0 {
        return Err(ContractError::TokenVerificationFailed);
    }

    // 2. Perform the zero-value self-transfer.
    //    Use `try_transfer` so a token that panics/reverts on zero-value transfer is
    //    caught as a typed error rather than propagating as an unhandled host panic.
    if token_client
        .try_transfer(&contract_addr, &contract_addr, &0_i128)
        .is_err()
    {
        return Err(ContractError::TokenRevertedOnZeroTransfer);
    }

    // 3. Read the balance again afterward.
    let final_balance = token_client.balance(&contract_addr);

    // Reject negative final balance.
    if final_balance < 0 {
        return Err(ContractError::TokenVerificationFailed);
    }

    // 4. Assert the balance is unchanged.
    if initial_balance != final_balance {
        return Err(ContractError::TokenVerificationFailed);
    }

    Ok(())
}
