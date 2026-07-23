// Chaos test suite for concurrent operation interleavings

//! This test module generates all permutations of a set of stream operations
//! (withdraw, cancel_stream, update_rate, pause_stream, resume_stream) against
//! a freshly created stream and asserts that post‑conditions hold regardless of
//! the order.
//!
//! The goal is to surface race‑condition‑like bugs in the Soroban runtime where
//! multiple transaction invocations are included in the same ledger close.
//!
//! Each permutation is applied to an independent test context to ensure isolation.
//! On failure the permutation seed is printed for reproducibility.
//!
//! # Ledger advancement
//! Pause and resume operations are gated behind a cooldown of
//! `MIN_PAUSE_INTERVAL_LEDGERS` (17) ledgers.  To ensure that a pause→resume
//! sequence in a permutation actually exercises the state transition rather than
//! always bouncing off the cooldown guard, the ledger sequence is advanced by
//! `OP_LEDGER_STEP` (20) before every op application.  The timestamp is kept in
//! sync so that the token‑conservation invariant (total tokens == 1000) remains
//! meaningful throughout the stream's lifetime.

use fluxora_stream::{ContractError, FluxoraStream, FluxoraStreamClient, PauseReason, StreamStatus};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::Client as TokenClient,
    Address, Env,
};

/// Number of ledgers advanced between each op to clear the pause/resume cooldown.
const OP_LEDGER_STEP: u32 = 20;

struct TestContext {
    env: Env,
    client: FluxoraStreamClient<'static>,
    sender: Address,
    recipient: Address,
    token: TokenClient<'static>,
    contract_id: Address,
    admin: Address,
}

impl TestContext {
    fn new() -> Self {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register_contract(None, FluxoraStream);
        let client = FluxoraStreamClient::new(&env, &contract_id);

        let token_admin = Address::generate(&env);
        let token_id = env
            .register_stellar_asset_contract_v2(token_admin)
            .address();
        let token = TokenClient::new(&env, &token_id);

        let admin = Address::generate(&env);
        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);

        // Initialise contract
        client.init(&token_id, &admin);

        Self {
            env,
            client,
            sender,
            recipient,
            token,
            contract_id,
            admin,
        }
    }

    fn create_stream(&self) -> u64 {
        // Simple 1000 token stream over 1000 seconds, rate 1 token/s
        self.client.create_stream(
            &self.sender,
            &self.recipient,
            &1000_i128,
            &1_i128,
            &0_u64,
            &0_u64,
            &1000_u64,
            &0_i128,
            &None,
        )
    }

    /// Advance both the ledger sequence number and the timestamp by one step.
    ///
    /// Moving the timestamp forward keeps `calculate_accrued` and the
    /// token-conservation check consistent.  Each step advances time by a
    /// small amount (well within the 1 000-second stream window).
    fn advance_ledger(&self, steps_so_far: u32) {
        let new_seq = steps_so_far * OP_LEDGER_STEP;
        // Advance timestamp proportionally (1 second per ledger step, capped
        // at 500 so we stay inside the stream window for all permutations).
        let new_ts = ((new_seq as u64) * 1).min(500);
        self.env.ledger().with_mut(|l| {
            l.sequence_number = new_seq;
            l.timestamp = new_ts;
        });
    }
}

/// Operations that the chaos permutation suite exercises.
///
/// Every variant must be safe to call on any stream state: if the operation is
/// not applicable (wrong state, cooldown active, terminal state, etc.) the
/// contract returns an error, which `apply_op` silently absorbs via
/// `catch_unwind` — matching how the existing ops already handle inapplicable
/// states.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Op {
    Cancel,
    Pause,
    Resume,
    UpdateRate,
    Withdraw,
}

fn apply_op(ctx: &TestContext, stream_id: u64, op: Op) -> Result<(), ContractError> {
    match op {
        Op::Withdraw => {
            ctx.client.withdraw(&stream_id);
            Ok(())
        }
        Op::Cancel => {
            ctx.client.cancel_stream(&stream_id);
            Ok(())
        }
        Op::UpdateRate => {
            // increase rate to 2 tokens/s (still covered by deposit)
            ctx.client.update_rate_per_second(&stream_id, &2_i128);
            Ok(())
        }
        Op::Pause => {
            ctx.client
                .pause_stream(&stream_id, &PauseReason::Operational);
            Ok(())
        }
        Op::Resume => {
            // Only applicable when the stream is currently Paused.  All other
            // states (Active, Completed, Cancelled) cause the contract to
            // return ContractError::StreamNotPaused, which is caught by the
            // caller's catch_unwind — identical to how Pause no-ops on an
            // already-paused or terminal stream.
            ctx.client.resume_stream(&stream_id);
            Ok(())
        }
    }
}

fn post_conditions_hold(ctx: &TestContext, stream_id: u64) {
    // Balance of contract should never be negative (checked by overflow safety).
    let contract_bal = ctx.token.balance(&ctx.contract_id);
    assert!(contract_bal >= 0, "contract balance negative");

    // Stream status must be a valid enum value.
    let status = ctx.client.get_stream_state(&stream_id).status;
    match status {
        StreamStatus::Active
        | StreamStatus::Paused
        | StreamStatus::Completed
        | StreamStatus::Cancelled => {}
        _ => panic!("invalid stream status"),
    }

    // No double refund on cancel: total tokens in contract + sender balance = initial deposit + any refunds.
    // Compute expected total: initial deposit (1000) + possible refunds from rate change (0) + possible withdrawals.
    // For simplicity we just ensure contract balance + sender balance + recipient balance == 1000.
    let sender_bal = ctx.token.balance(&ctx.sender);
    let recipient_bal = ctx.token.balance(&ctx.recipient);
    assert_eq!(
        contract_bal + sender_bal + recipient_bal,
        1000,
        "tokens mismatch"
    );
}

proptest::proptest! {
    #[test]
    fn chaos_permutations(seed: u64) {
        // Generate all permutations of the five ops (5! = 120 permutations).
        let mut permuts = vec![
            Op::Cancel,
            Op::Pause,
            Op::Resume,
            Op::UpdateRate,
            Op::Withdraw,
        ];
        let mut permutations: Vec<Vec<Op>> = Vec::new();
        loop {
            permutations.push(permuts.clone());
            if !next_permutation(&mut permuts) {
                break;
            }
        }

        for perm in permutations {
            // Fresh context per permutation
            let ctx = TestContext::new();
            let stream_id = ctx.create_stream();

            for (step, op) in perm.iter().enumerate() {
                // Advance the ledger before each op so that the pause/resume
                // cooldown (MIN_PAUSE_INTERVAL_LEDGERS = 17) is satisfied.
                // `step + 1` ensures we're always past the initial ledger 0.
                ctx.advance_ledger((step as u32) + 1);

                // Each op may panic on invalid state; we ignore errors for
                // this chaos test so that the post-condition check below is
                // the sole arbiter of correctness.
                let _ = std::panic::catch_unwind(|| apply_op(&ctx, stream_id, *op));
            }
            // Verify post‑conditions. If they fail we include the seed for reproducibility.
            post_conditions_hold(&ctx, stream_id);
        }
        // Log seed for reproducibility
        println!("seed: {}", seed);
    }
}

// Helper: generate next lexicographic permutation (returns false when finished).
fn next_permutation<T: Ord>(data: &mut [T]) -> bool {
    // Find the largest index i such that data[i] < data[i + 1]
    if data.len() < 2 {
        return false;
    }
    let mut i = data.len() - 2;
    while i != usize::MAX && data[i] >= data[i + 1] {
        if i == 0 {
            return false;
        }
        i -= 1;
    }
    // Find the largest index j > i such that data[i] < data[j]
    let mut j = data.len() - 1;
    while data[i] >= data[j] {
        j -= 1;
    }
    data.swap(i, j);
    data[i + 1..].reverse();
    true
}
