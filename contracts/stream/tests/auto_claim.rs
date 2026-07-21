extern crate std;

use fluxora_stream::{
    AutoClaimStatus, ContractError, FluxoraStream, FluxoraStreamClient, StreamKind,
};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    Address, Env,
};

struct Ctx<'a> {
    env: Env,
    client: FluxoraStreamClient<'a>,
    contract_id: Address,
    sender: Address,
    recipient: Address,
    token: TokenClient<'a>,
}

impl<'a> Ctx<'a> {
    fn setup() -> Self {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register_contract(None, FluxoraStream);
        let client = FluxoraStreamClient::new(&env, &contract_id);

        let token_admin = Address::generate(&env);
        let token_id = env
            .register_stellar_asset_contract_v2(token_admin)
            .address();
        let stellar_asset = StellarAssetClient::new(&env, &token_id);
        let token = TokenClient::new(&env, &token_id);

        let admin = Address::generate(&env);
        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);

        stellar_asset.mint(&sender, &1_000_000_000_000i128);
        token.approve(&sender, &contract_id, &i128::MAX, &100_000u32);

        client.init(&token_id, &admin);

        Self {
            env,
            client,
            contract_id,
            sender,
            recipient,
            token,
        }
    }

    fn create_default_stream(&self) -> u64 {
        let now = self.env.ledger().timestamp();
        self.client.create_stream(
            &self.sender,
            &self.recipient,
            &1000i128,     // deposit
            &1i128,        // rate
            &(now + 1),    // start_time
            &(now + 1),    // cliff_time
            &(now + 1001), // end_time (duration = 1000s)
            &0i128,        // fee
            &None,         // template_id
            &StreamKind::Linear,
        )
    }
}

// ---------------------------------------------------------------------------
// Auto-Claim revoke, races, destination updates, and timing tests
// ---------------------------------------------------------------------------

/// Revoke Boundary Semantics:
/// - A stream recipient can set or update their auto-claim destination at any point during
///   the active stream lifecycle.
/// - Revocation completely removes the stored destination key `AutoClaimDestination(stream_id)`.
/// - After revocation, any attempts to trigger auto-claim are blocked with `ContractError::InvalidParams`
///   without transferring any tokens, leaving all balances unaffected.
///
/// Early Trigger Semantics:
/// - Auto-claim is a permissionless mechanism intended to execute the final settlement at stream completion.
/// - Triggering auto-claim is strictly disallowed before the stream's `end_time` is reached.
/// - Early trigger attempts revert with `ContractError::InvalidState` and perform no token transfers.
///
/// Destination Change Semantics:
/// - The recipient can overwrite their auto-claim destination. The update is immediately reflected.
/// - Triggering auto-claim after an update guarantees funds are transferred ONLY to the recipient's
///   most recently chosen destination. Any prior destinations receive zero tokens.
///
/// Recipient-Controlled Destination Security:
/// - Auto-claim destination configuration/revocation requires explicit recipient authorization.
/// - Since only the recipient can configure where funds are sent, third-party triggers cannot direct
///   tokens to unauthorized addresses, preserving recipient control and security.

#[test]
fn test_auto_claim_revoke_then_trigger() {
    let ctx = Ctx::setup();
    ctx.env.ledger().set_timestamp(1_000_000);
    let stream_id = ctx.create_default_stream();
    let destination = Address::generate(&ctx.env);

    // 1. Configure auto-claim
    ctx.client.set_auto_claim(&stream_id, &destination);
    assert_eq!(
        ctx.client.get_auto_claim_destination(&stream_id),
        Some(destination.clone())
    );

    // 2. Revoke auto-claim
    ctx.client.revoke_auto_claim(&stream_id);
    assert_eq!(ctx.client.get_auto_claim_destination(&stream_id), None);

    // 3. Fast-forward to end time (past stream completion)
    ctx.env.ledger().set_timestamp(1_001_001);

    // 4. Attempt to trigger auto-claim (must fail)
    let contract_bal_before = ctx.token.balance(&ctx.contract_id);
    let dest_bal_before = ctx.token.balance(&destination);

    let result = ctx.client.try_trigger_auto_claim(&stream_id);
    assert_eq!(result, Err(Ok(ContractError::InvalidParams)));

    // 5. Verify no transfer occurred and balances remain unchanged
    assert_eq!(ctx.token.balance(&ctx.contract_id), contract_bal_before);
    assert_eq!(ctx.token.balance(&destination), dest_bal_before);
}

#[test]
fn test_auto_claim_trigger_before_eligibility() {
    let ctx = Ctx::setup();
    ctx.env.ledger().set_timestamp(1_000_000);
    let stream_id = ctx.create_default_stream();
    let destination = Address::generate(&ctx.env);

    // 1. Configure auto-claim
    ctx.client.set_auto_claim(&stream_id, &destination);

    // 2. Attempt to trigger before end_time (at now + 500s)
    ctx.env.ledger().set_timestamp(1_000_500);

    let contract_bal_before = ctx.token.balance(&ctx.contract_id);
    let dest_bal_before = ctx.token.balance(&destination);

    let result = ctx.client.try_trigger_auto_claim(&stream_id);
    assert_eq!(result, Err(Ok(ContractError::InvalidState)));

    // 3. Verify no funds were transferred
    assert_eq!(ctx.token.balance(&ctx.contract_id), contract_bal_before);
    assert_eq!(ctx.token.balance(&destination), dest_bal_before);
}

#[test]
fn test_auto_claim_destination_update() {
    let ctx = Ctx::setup();
    ctx.env.ledger().set_timestamp(1_000_000);
    let stream_id = ctx.create_default_stream();
    let destination_a = Address::generate(&ctx.env);
    let destination_b = Address::generate(&ctx.env);

    // 1. Configure with destination A
    ctx.client.set_auto_claim(&stream_id, &destination_a);
    assert_eq!(
        ctx.client.get_auto_claim_destination(&stream_id),
        Some(destination_a.clone())
    );

    // 2. Update to destination B
    ctx.client.set_auto_claim(&stream_id, &destination_b);
    assert_eq!(
        ctx.client.get_auto_claim_destination(&stream_id),
        Some(destination_b.clone())
    );

    // 3. Fast-forward to/past end time
    ctx.env.ledger().set_timestamp(1_001_001);

    let dest_a_bal_before = ctx.token.balance(&destination_a);
    let dest_b_bal_before = ctx.token.balance(&destination_b);
    let contract_bal_before = ctx.token.balance(&ctx.contract_id);

    // 4. Trigger auto-claim
    let amount = ctx.client.trigger_auto_claim(&stream_id);
    assert_eq!(amount, 1000);

    // 5. Verify funds are sent ONLY to destination B
    assert_eq!(ctx.token.balance(&destination_b), dest_b_bal_before + 1000);
    assert_eq!(ctx.token.balance(&destination_a), dest_a_bal_before);
    assert_eq!(
        ctx.token.balance(&ctx.contract_id),
        contract_bal_before - 1000
    );
}

#[test]
fn test_auto_claim_status_consistency() {
    let ctx = Ctx::setup();
    ctx.env.ledger().set_timestamp(1_000_000);
    let stream_id = ctx.create_default_stream();
    let destination_a = Address::generate(&ctx.env);
    let destination_b = Address::generate(&ctx.env);

    // 1. Initial status: NotSet
    assert_eq!(
        ctx.client.get_auto_claim_status(&stream_id),
        AutoClaimStatus::NotSet
    );
    assert_eq!(ctx.client.get_auto_claim_destination(&stream_id), None);

    // 2. Configure auto-claim -> Active (ValidDestination) with A
    ctx.client.set_auto_claim(&stream_id, &destination_a);
    let status1 = ctx.client.get_auto_claim_status(&stream_id);
    if let AutoClaimStatus::ValidDestination(payload) = status1 {
        assert_eq!(payload.destination, destination_a);
        assert_eq!(payload.claimable, 0); // At timestamp 1_000_000 (0s elapsed since start)
    } else {
        panic!("expected ValidDestination");
    }
    assert_eq!(
        ctx.client.get_auto_claim_destination(&stream_id),
        Some(destination_a.clone())
    );

    // 3. Update auto-claim -> Active (ValidDestination) with B
    ctx.client.set_auto_claim(&stream_id, &destination_b);
    ctx.env.ledger().set_timestamp(1_000_500); // 500 seconds elapsed (499 accrued)
    let status2 = ctx.client.get_auto_claim_status(&stream_id);
    if let AutoClaimStatus::ValidDestination(payload) = status2 {
        assert_eq!(payload.destination, destination_b);
        assert_eq!(payload.claimable, 499); // start_time is 1_000_001, so 499s elapsed at 1_000_500
    } else {
        panic!("expected ValidDestination");
    }
    assert_eq!(
        ctx.client.get_auto_claim_destination(&stream_id),
        Some(destination_b.clone())
    );

    // 4. Revoke auto-claim -> NotSet
    ctx.client.revoke_auto_claim(&stream_id);
    assert_eq!(
        ctx.client.get_auto_claim_status(&stream_id),
        AutoClaimStatus::NotSet
    );
    assert_eq!(ctx.client.get_auto_claim_destination(&stream_id), None);
}
