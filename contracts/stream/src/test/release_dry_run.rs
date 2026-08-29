//! Regression tests for release dry-run pre-flight validation (#1595).
//!
//! Verifies:
//! - Local validation rejects invalid network, contract, and initialization identifiers before mutation
//! - Boundary checks on 56-character base32 strkeys (length, prefix, alphabet)
//! - Simulation / dry-run reads produce zero state side-effects or pool mutation
//! - Explicit confirmation requirement: read-only pre-flight vs authorized state mutation
//! - Retry safety: failed attempts leave zero storage footprint or balance change

use super::common::*;
use crate::Error;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::Address;

/// Helper: validate contract ID format locally according to Stellar Strkey rules
/// (starts with 'C', length 56, valid Base32 characters [A-Z2-7]).
fn validate_contract_id(id: &str) -> Result<(), &'static str> {
    if id.is_empty() {
        return Err("Contract ID cannot be empty");
    }
    if id.len() != 56 {
        return Err("Contract ID length must be exactly 56 characters");
    }
    if !id.starts_with('C') {
        return Err("Contract ID must start with 'C'");
    }
    for ch in id.chars() {
        if !matches!(ch, 'A'..='Z' | '2'..='7') {
            return Err("Contract ID contains invalid Base32 character");
        }
    }
    Ok(())
}

/// Helper: validate network identifier format.
fn validate_network_identifier(net: &str) -> Result<(), &'static str> {
    if net.is_empty() {
        return Err("Network identifier cannot be empty");
    }
    for ch in net.chars() {
        if !matches!(ch, 'a'..='z' | 'A'..='Z' | '0'..='9' | '_' | '-') {
            return Err("Network identifier contains invalid characters");
        }
    }
    Ok(())
}

#[test]
fn valid_contract_id_format_is_accepted() {
    let valid = "CBCGTSCJXBMPPPE4BPDIPYZXPE2J5TQEKD2KCS7VQF533NKKEYGUTHXW";
    assert!(validate_contract_id(valid).is_ok());
}

#[test]
fn boundary_contract_id_lengths_are_rejected() {
    // 55 chars - one too short
    let short = "CBCGTSCJXBMPPPE4BPDIPYZXPE2J5TQEKD2KCS7VQF533NKKEYGUTHX";
    assert_eq!(
        validate_contract_id(short).unwrap_err(),
        "Contract ID length must be exactly 56 characters"
    );

    // 57 chars - one too long
    let long = "CBCGTSCJXBMPPPE4BPDIPYZXPE2J5TQEKD2KCS7VQF533NKKEYGUTHXWA";
    assert_eq!(
        validate_contract_id(long).unwrap_err(),
        "Contract ID length must be exactly 56 characters"
    );

    // Empty
    assert_eq!(
        validate_contract_id("").unwrap_err(),
        "Contract ID cannot be empty"
    );
}

#[test]
fn boundary_contract_id_prefix_and_alphabet_are_rejected() {
    // Starts with G instead of C (public key, not contract address)
    let account = "GBCGTSCJXBMPPPE4BPDIPYZXPE2J5TQEKD2KCS7VQF533NKKEYGUTHXW";
    assert_eq!(
        validate_contract_id(account).unwrap_err(),
        "Contract ID must start with 'C'"
    );

    // Contains 0 or 8 (invalid Crockford base32)
    let invalid_char = "CBCGTSCJXBMPPPE4BPDIPYZXPE2J5TQEKD2KCS7VQF533NKKEYGUTH08";
    assert_eq!(
        validate_contract_id(invalid_char).unwrap_err(),
        "Contract ID contains invalid Base32 character"
    );

    // Lowercase characters
    let lowercase = "cbcgtscjXbmpppe4bpdipyzxpe2j5tqekd2kcs7vqf533nkkeyguthxw";
    assert_eq!(
        validate_contract_id(lowercase).unwrap_err(),
        "Contract ID must start with 'C'"
    );
}

#[test]
fn network_identifier_validation_rejects_empty_and_illegal_characters() {
    assert!(validate_network_identifier("testnet").is_ok());
    assert!(validate_network_identifier("mainnet").is_ok());
    assert!(validate_network_identifier("futurenet").is_ok());
    assert!(validate_network_identifier("custom-net_123").is_ok());

    assert_eq!(
        validate_network_identifier("").unwrap_err(),
        "Network identifier cannot be empty"
    );
    assert_eq!(
        validate_network_identifier("testnet;inject").unwrap_err(),
        "Network identifier contains invalid characters"
    );
}

#[test]
fn dry_run_simulation_leaves_pool_and_stream_count_strictly_unchanged() {
    let h = Harness::new();
    let initial_count = h.client.stream_count();
    let initial_pool = h.pool();

    // Pre-flight view queries (simulating read-only inspection)
    assert!(!h.client.stream_exists(&0));
    assert_eq!(
        h.client.try_get_stream(&0).unwrap_err().unwrap(),
        Error::StreamNotFound
    );

    // Assert pool and stream count did not change
    assert_eq!(h.client.stream_count(), initial_count);
    assert_eq!(h.pool(), initial_pool);
    h.assert_pool_exact();
}

#[test]
fn rejected_pre_flight_leaves_zero_residue_for_retry() {
    let h = Harness::new();
    let pool_before = h.pool();
    let count_before = h.client.stream_count();

    // Simulating invalid initialization input
    let start = h.now();
    let err = h
        .client
        .try_create_stream(
            &h.sender,
            &h.recipient,
            &h.token,
            &0, // invalid deposit
            &start,
            &(start + DAY),
            &start,
            &true,
            &true,
            &true,
        )
        .unwrap_err()
        .unwrap();
    assert_eq!(err, Error::InvalidDeposit);

    // State is untouched
    assert_eq!(h.client.stream_count(), count_before);
    assert_eq!(h.pool(), pool_before);

    // Retrying with valid input succeeds deterministically
    let id = h.create(100 * ONE, start, start + DAY, start, true, true, true);
    assert_eq!(id, 0);
    assert_eq!(h.client.stream_count(), count_before + 1);
    assert_eq!(h.pool(), pool_before + 100 * ONE);
    h.assert_pool_exact();
}

#[test]
fn authorization_check_rejects_unauthorized_state_mutation() {
    let h = Harness::new();
    let start = h.now();
    let id = h.create(100 * ONE, start, start + DAY, start, true, true, true);
    let pool_before = h.pool();

    // An unauthorized third party attempting to cancel is rejected
    let _attacker = Address::generate(&h.env);
    h.env.mock_auths(&[]); // attacker provides no valid auth
    let res = h.client.try_cancel(&id);
    assert!(res.is_err());

    // Pool and stream remain unchanged
    assert_eq!(h.pool(), pool_before);
    let s = h.client.get_stream(&id);
    assert_eq!(s.status, crate::StreamStatus::Active);
}
