extern crate std;

use ed25519_dalek::{Signer, SigningKey};
use fluxora_stream::{ContractError, FluxoraStream, FluxoraStreamClient, StreamKind, StreamStatus};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::Client as TokenClient,
    Address, BytesN, Env,
};
use soroban_sdk::xdr::{AccountId, PublicKey, ScAddress, Uint256};

// ---------------------------------------------------------------------------
// Test context
// ---------------------------------------------------------------------------

struct TestContext<'a> {
    env: Env,
    client: FluxoraStreamClient<'a>,
    sender: Address,
    recipient: Address,
    token: TokenClient<'a>,
    contract_id: Address,
    attester_key: SigningKey,
    attester_pk: BytesN<32>,
}

impl<'a> TestContext<'a> {
    fn setup() -> Self {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register_contract(None, FluxoraStream);
        let client = FluxoraStreamClient::new(&env, &contract_id);

        let token_admin = Address::generate(&env);
        let token_id = env
            .register_stellar_asset_contract_v2(token_admin)
            .address();
        let admin = Address::generate(&env);
        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);

        client.init(&token_id, &admin);

        let sac = soroban_sdk::token::StellarAssetClient::new(&env, &token_id);
        sac.mint(&sender, &10_000_i128);
        TokenClient::new(&env, &token_id).approve(&sender, &contract_id, &i128::MAX, &100_000);

        let attester_key = SigningKey::from_bytes(&[0xABu8; 32]);
        let pk_arr = attester_key.verifying_key().to_bytes();
        let attester_pk = BytesN::from_array(&env, &pk_arr);

        Self {
            env,
            client,
            sender,
            recipient,
            token,
            contract_id,
            attester_key,
            attester_pk,
        }
    }

    fn create_cliff_only_stream(&self, deposit: i128, start: u64, cliff: u64, end: u64) -> u64 {
        self.client.create_stream(
            &self.sender,
            &self.recipient,
            &deposit,
            &0_i128,
            &start,
            &cliff,
            &end,
            &0,
            &None,
            &StreamKind::CliffOnly,
        )
    }

    fn pk_to_address(env: &Env, pk: &[u8; 32]) -> Address {
        ScAddress::Account(AccountId(PublicKey::PublicKeyTypeEd25519(Uint256(*pk))))
            .try_into_val(env)
            .expect("valid ed25519 key → address")
    }

    fn build_milestone_msg(env: &Env, stream_id: u64, milestone_id: u64, deadline: u64) -> soroban_sdk::Bytes {
        let mut msg = soroban_sdk::Bytes::new(env);
        msg.extend_from_slice(b"FluxoraMilestoneProof");
        msg.extend_from_array(&stream_id.to_be_bytes());
        msg.extend_from_array(&milestone_id.to_be_bytes());
        msg.extend_from_array(&deadline.to_be_bytes());
        msg
    }

fn sign_milestone_msg(env: &Env, signing_key: &SigningKey, msg: &soroban_sdk::Bytes) -> BytesN<64> {
    let bytes: std::vec::Vec<u8> = (0..msg.len()).map(|i| msg.get_unchecked(i)).collect();
    BytesN::from_array(env, &signing_key.sign(&bytes).to_bytes())
}
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn test_verify_milestone_proof_success() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let stream_id = ctx.create_cliff_only_stream(1000, 0, 500, 1000);

    // Set attester allowlist
    let attesters = soroban_sdk::Vec::new(&ctx.env);
    attesters.push_back(&ctx.attester_pk);
    ctx.client.set_attester_allowlist(&ctx.sender, &stream_id, &attesters);

    // Sign a milestone proof
    let milestone_id = 1u64;
    let deadline = 1000u64;
    let msg = TestContext::build_milestone_msg(&ctx.env, stream_id, milestone_id, deadline);
    let signature = TestContext::sign_milestone_msg(&ctx.env, &ctx.attester_key, &msg);

    // Verify the proof
    let result = ctx.client.verify_milestone_proof(
        &ctx.attester_pk,
        &signature,
        &stream_id,
        &milestone_id,
        &deadline,
    );
    assert!(result.is_ok(), "verify_milestone_proof should succeed");

    // Verify early_cliff_unlocked is set
    let stream = ctx.client.get_stream_state(&stream_id);
    assert!(stream.early_cliff_unlocked, "early_cliff_unlocked should be true");
}

#[test]
fn test_verify_milestone_proof_expired_deadline() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(100);

    let stream_id = ctx.create_cliff_only_stream(1000, 0, 500, 1000);

    let attesters = soroban_sdk::Vec::new(&ctx.env);
    attesters.push_back(&ctx.attester_pk);
    ctx.client.set_attester_allowlist(&ctx.sender, &stream_id, &attesters);

    let milestone_id = 1u64;
    let deadline = 50u64;
    let msg = TestContext::build_milestone_msg(&ctx.env, stream_id, milestone_id, deadline);
    let signature = TestContext::sign_milestone_msg(&ctx.env, &ctx.attester_key, &msg);

    let result = ctx.client.verify_milestone_proof(
        &ctx.attester_pk,
        &signature,
        &stream_id,
        &milestone_id,
        &deadline,
    );
    assert_eq!(result, Err(ContractError::AttestationExpired));
}

#[test]
fn test_verify_milestone_proof_replay() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let stream_id = ctx.create_cliff_only_stream(1000, 0, 500, 1000);

    let attesters = soroban_sdk::Vec::new(&ctx.env);
    attesters.push_back(&ctx.attester_pk);
    ctx.client.set_attester_allowlist(&ctx.sender, &stream_id, &attesters);

    let milestone_id = 1u64;
    let deadline = 1000u64;
    let msg = TestContext::build_milestone_msg(&ctx.env, stream_id, milestone_id, deadline);
    let signature = TestContext::sign_milestone_msg(&ctx.env, &ctx.attester_key, &msg);

    // First call succeeds
    let result = ctx.client.verify_milestone_proof(
        &ctx.attester_pk,
        &signature,
        &stream_id,
        &milestone_id,
        &deadline,
    );
    assert!(result.is_ok());

    // Second call with same milestone_id must fail
    let result2 = ctx.client.verify_milestone_proof(
        &ctx.attester_pk,
        &signature,
        &stream_id,
        &milestone_id,
        &deadline,
    );
    assert_eq!(result2, Err(ContractError::AttestationReplayed));
}

#[test]
fn test_verify_milestone_proof_unauthorized_attester() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let stream_id = ctx.create_cliff_only_stream(1000, 0, 500, 1000);

    // Allowlist is empty — no attesters authorized
    let attesters = soroban_sdk::Vec::new(&ctx.env);
    ctx.client.set_attester_allowlist(&ctx.sender, &stream_id, &attesters);

    let milestone_id = 1u64;
    let deadline = 1000u64;
    let msg = TestContext::build_milestone_msg(&ctx.env, stream_id, milestone_id, deadline);
    let signature = TestContext::sign_milestone_msg(&ctx.env, &ctx.attester_key, &msg);

    let result = ctx.client.verify_milestone_proof(
        &ctx.attester_pk,
        &signature,
        &stream_id,
        &milestone_id,
        &deadline,
    );
    assert_eq!(result, Err(ContractError::UnauthorizedAttester));
}

#[test]
fn test_verify_milestone_proof_not_cliff_only() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let stream_id = ctx.create_cliff_only_stream(1000, 0, 500, 1000);

    let attesters = soroban_sdk::Vec::new(&ctx.env);
    attesters.push_back(&ctx.attester_pk);
    ctx.client.set_attester_allowlist(&ctx.sender, &stream_id, &attesters);

    // Create a Linear stream
    let linear_id = ctx.client.create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &0,
        &None,
        &StreamKind::Linear,
    );

    let attesters2 = soroban_sdk::Vec::new(&ctx.env);
    attesters2.push_back(&ctx.attester_pk);
    ctx.client.set_attester_allowlist(&ctx.sender, &linear_id, &attesters2);

    let milestone_id = 1u64;
    let deadline = 1000u64;
    let msg = TestContext::build_milestone_msg(&ctx.env, linear_id, milestone_id, deadline);
    let signature = TestContext::sign_milestone_msg(&ctx.env, &ctx.attester_key, &msg);

    let result = ctx.client.verify_milestone_proof(
        &ctx.attester_pk,
        &signature,
        &linear_id,
        &milestone_id,
        &deadline,
    );
    assert_eq!(result, Err(ContractError::StreamNotCliffOnly));
}

#[test]
fn test_verify_milestone_proof_terminal_stream() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let stream_id = ctx.create_cliff_only_stream(1000, 0, 500, 1000);

    let attesters = soroban_sdk::Vec::new(&ctx.env);
    attesters.push_back(&ctx.attester_pk);
    ctx.client.set_attester_allowlist(&ctx.sender, &stream_id, &attesters);

    // Cancel the stream first
    ctx.client.cancel_stream(&ctx.sender, &stream_id);

    let milestone_id = 1u64;
    let deadline = 1000u64;
    let msg = TestContext::build_milestone_msg(&ctx.env, stream_id, milestone_id, deadline);
    let signature = TestContext::sign_milestone_msg(&ctx.env, &ctx.attester_key, &msg);

    let result = ctx.client.verify_milestone_proof(
        &ctx.attester_pk,
        &signature,
        &stream_id,
        &milestone_id,
        &deadline,
    );
    assert_eq!(result, Err(ContractError::InvalidState));
}

#[test]
fn test_verify_milestone_proof_invalid_signature() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let stream_id = ctx.create_cliff_only_stream(1000, 0, 500, 1000);

    let attesters = soroban_sdk::Vec::new(&ctx.env);
    attesters.push_back(&ctx.attester_pk);
    ctx.client.set_attester_allowlist(&ctx.sender, &stream_id, &attesters);

    let milestone_id = 1u64;
    let deadline = 1000u64;
    let msg = TestContext::build_milestone_msg(&ctx.env, stream_id, milestone_id, deadline);

    // Sign with a different key
    let wrong_key = SigningKey::from_bytes(&[0xCDu8; 32]);
    let wrong_sig = TestContext::sign_milestone_msg(&ctx.env, &wrong_key, &msg);

    let result = ctx.client.verify_milestone_proof(
        &ctx.attester_pk,
        &wrong_sig,
        &stream_id,
        &milestone_id,
        &deadline,
    );
    // ed25519_verify traps on invalid signature; in the test env this
    // should return an error (host error mapped to InvalidAttestation).
    assert!(result.is_err());
}

#[test]
fn test_set_attester_allowlist_non_sender() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let stream_id = ctx.create_cliff_only_stream(1000, 0, 500, 1000);

    let other = Address::generate(&ctx.env);
    let attesters = soroban_sdk::Vec::new(&ctx.env);
    attesters.push_back(&ctx.attester_pk);

    let result = ctx.client.set_attester_allowlist(&other, &stream_id, &attesters);
    assert_eq!(result, Err(ContractError::Unauthorized));
}

#[test]
fn test_set_attester_allowlist_linear_stream() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let stream_id = ctx.client.create_stream(
        &ctx.sender,
        &ctx.recipient,
        &1000_i128,
        &1_i128,
        &0u64,
        &0u64,
        &1000u64,
        &0,
        &None,
        &StreamKind::Linear,
    );

    let attesters = soroban_sdk::Vec::new(&ctx.env);
    attesters.push_back(&ctx.attester_pk);

    let result = ctx.client.set_attester_allowlist(&ctx.sender, &stream_id, &attesters);
    assert_eq!(result, Err(ContractError::StreamNotCliffOnly));
}