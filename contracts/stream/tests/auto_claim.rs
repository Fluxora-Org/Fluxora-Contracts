extern crate std;

use fluxora_stream::{
    AutoClaimStatus, AutoClaimValidPayload, ContractError, FluxoraStream, FluxoraStreamClient,
    StreamKind,
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

// ---------------------------------------------------------------------------
// Immediate re-trigger / no-op path
// ---------------------------------------------------------------------------

/// Verifies that calling `trigger_auto_claim` a second time immediately after a
/// successful first call is cleanly rejected as a no-op.
///
/// ## Behavioural contract
/// - First call: accrued tokens are transferred to the destination; stream
///   transitions to `Completed`; returns the withdrawn amount.
/// - Second call (immediate): the terminal-status guard (`Completed`) fires
///   **before** any computation, token lookup, or state mutation.  The call
///   returns `ContractError::InvalidState` and emits zero events.
/// - Balances are entirely unaffected by the second call.
///
/// ## Status accuracy across repeated triggers
/// After the first successful auto-claim the destination key is **still
/// present** in persistent storage (the contract never purges it on
/// completion).  Therefore `get_auto_claim_status` returns
/// `ValidDestination { destination, claimable: 0 }` — indicating that the
/// destination is registered but there is nothing left to withdraw.  This
/// accurately reflects reality and lets any off-chain keeper distinguish "not
/// set up" (NotSet) from "already claimed" (ValidDestination with claimable 0).
///
/// ## Gas-cost concern (reported, not patched — per issue guidelines)
/// The second trigger is **cheap** but not a zero-cost pure no-op.  Before the
/// terminal-status guard fires, `load_stream` executes a persistent storage
/// read (`env.storage().persistent().get(…)`).  On Soroban that read carries a
/// non-trivial CPU-instruction and I/O fee.  The no-op path is therefore
/// meaningfully cheaper than the working path (no accrual calculation, no
/// token transfer, no events), but operators who experience many spurious
/// re-triggers will still burn ledger fees for that initial storage read.
/// A future optimisation could add an in-memory cache or a lightweight
/// "already-claimed" flag that avoids the full `load_stream` on the hot path.
#[test]
fn test_immediate_retrigger_is_noop() {
    let ctx = Ctx::setup();
    ctx.env.ledger().set_timestamp(1_000_000);
    let stream_id = ctx.create_default_stream();

    // The stream has deposit = 1000, rate = 1 tok/s, start_time = 1_000_001,
    // end_time = 1_001_001, so it fully vests 1000 tokens over 1000 seconds.
    let destination = Address::generate(&ctx.env);
    ctx.client.set_auto_claim(&stream_id, &destination);

    // ── Pre-conditions ──────────────────────────────────────────────────────
    // Status should be ValidDestination with claimable == 0 (stream hasn't
    // started yet — we're still at 1_000_000, before start_time 1_000_001).
    let pre_status = ctx.client.get_auto_claim_status(&stream_id);
    assert_eq!(
        pre_status,
        AutoClaimStatus::ValidDestination(AutoClaimValidPayload {
            destination: destination.clone(),
            claimable: 0,
        }),
        "before stream start: claimable should be 0"
    );

    // Fast-forward to just past end_time so the full deposit has vested.
    ctx.env.ledger().set_timestamp(1_001_002);

    let contract_bal_before_first = ctx.token.balance(&ctx.contract_id);
    let dest_bal_before_first = ctx.token.balance(&destination);

    // ── First trigger: should succeed and transfer 1000 tokens ─────────────
    let first_result = ctx.client.trigger_auto_claim(&stream_id);
    assert_eq!(first_result, 1000, "first trigger must transfer full deposit");

    let contract_bal_after_first = ctx.token.balance(&ctx.contract_id);
    let dest_bal_after_first = ctx.token.balance(&destination);

    assert_eq!(
        dest_bal_after_first,
        dest_bal_before_first + 1000,
        "destination should receive 1000 tokens after first trigger"
    );
    assert_eq!(
        contract_bal_after_first,
        contract_bal_before_first - 1000,
        "contract escrow should decrease by 1000 after first trigger"
    );

    // ── Status after first trigger ──────────────────────────────────────────
    // The destination key is still stored; claimable is now 0 because
    // withdrawn_amount == deposit_amount.
    let status_after_first = ctx.client.get_auto_claim_status(&stream_id);
    assert_eq!(
        status_after_first,
        AutoClaimStatus::ValidDestination(AutoClaimValidPayload {
            destination: destination.clone(),
            claimable: 0,
        }),
        "after first trigger: destination still set, claimable must be 0"
    );

    // ── Second trigger: must be a no-op ────────────────────────────────────
    // The stream is now Completed; trigger_auto_claim must reject immediately
    // with InvalidState (terminal-status guard) without moving any tokens.
    let second_result = ctx.client.try_trigger_auto_claim(&stream_id);
    assert_eq!(
        second_result,
        Err(Ok(ContractError::InvalidState)),
        "second (immediate re-)trigger must return InvalidState"
    );

    // Balances must be identical to those captured right after the first call.
    assert_eq!(
        ctx.token.balance(&ctx.contract_id),
        contract_bal_after_first,
        "contract balance must not change on the no-op second trigger"
    );
    assert_eq!(
        ctx.token.balance(&destination),
        dest_bal_after_first,
        "destination balance must not change on the no-op second trigger"
    );

    // ── Status after second trigger ─────────────────────────────────────────
    // Status must remain unchanged: ValidDestination with claimable == 0.
    // This accurately reflects "claimed" rather than "not configured".
    let status_after_second = ctx.client.get_auto_claim_status(&stream_id);
    assert_eq!(
        status_after_second,
        AutoClaimStatus::ValidDestination(AutoClaimValidPayload {
            destination: destination.clone(),
            claimable: 0,
        }),
        "status must remain ValidDestination/claimable=0 after the no-op retrigger"
    );

    // ── Gas-cost observation ────────────────────────────────────────────────
    // Measure CPU cost of the no-op second trigger for documentation purposes.
    // The test does NOT assert a hard ceiling — that is left for gas_regression.rs.
    // Instead we print the cost so reviewers can track regressions manually.
    //
    // Expected: the no-op path (load_stream + status check + early return) is
    // substantially cheaper than the working path (load_stream + accrual calc +
    // state write + token transfer + event emission).
    ctx.env.budget().reset_unlimited();
    let _ = ctx.client.try_trigger_auto_claim(&stream_id);
    let noop_cpu_cost = ctx.env.budget().cpu_instruction_cost();
    println!(
        "GAS_OBSERVATION: trigger_auto_claim (no-op / already-completed): {} CPU instructions",
        noop_cpu_cost
    );

    // Sanity guard: no-op must consume fewer instructions than a plausible
    // upper bound for a full working trigger.  10 000 000 instructions is a
    // conservative ceiling; the real working path is typically several orders
    // of magnitude higher.  If this ever fails, something very unexpected has
    // changed on the no-op path and warrants investigation.
    assert!(
        noop_cpu_cost < 10_000_000,
        "no-op retrigger cost ({} instructions) unexpectedly high — investigate the rejection path",
        noop_cpu_cost
    );
}
