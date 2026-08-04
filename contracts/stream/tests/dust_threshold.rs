//! Dust-threshold boundary tests aligned with `docs/dust-threshold.md`.
//!
//! Documents the enforcement formula:
//!   withdrawable < withdraw_dust_threshold  →  return 0 (blocked)
//!   withdrawable == threshold               →  allowed (strict `<`, not `<=`)
//!   withdrawable > threshold                →  allowed
//!
//! Covers every public entry point that consults the dust check:
//!   `withdraw`, `withdraw_to`, `batch_withdraw`, `batch_withdraw_to`.
//!
//! ## Doc / code mismatches (flagged, not silently fixed)
//!
//! 1. **`InvalidDustThreshold` (code 35) is enforced on `create_stream_offer` only.**
//!    `create_stream`, `create_streams`, and template/relative creation paths do
//!    **not** validate `withdraw_dust_threshold` bounds. See
//!    `flag_mismatch_create_allows_threshold_above_deposit` and
//!    `flag_mismatch_create_allows_negative_dust_threshold`.
//!
//! 2. **`delegated_withdraw` does not consult the dust threshold.**
//!    Only `withdraw`, `withdraw_to`, `batch_withdraw`, and `batch_withdraw_to`
//!    apply the check documented in `docs/dust-threshold.md`.
//!
//! 3. **`token_check.rs` does not implement dust-threshold logic.**
//!    Zero-amount SEP-41 smoke tests live in `token_check::verify_token_behavior`
//!    (covered in `src/test_token_edge_cases.rs`). Dust enforcement lives in the
//!    withdraw paths in `lib.rs`.

extern crate std;

use fluxora_stream::{
    CreateStreamParams, FluxoraStream, FluxoraStreamClient, StreamKind, WithdrawToParam,
};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    vec, Address, Env,
};

struct TestContext<'a> {
    env: Env,
    contract_id: Address,
    sender: Address,
    recipient: Address,
    token: TokenClient<'a>,
}

impl<'a> TestContext<'a> {
    fn setup() -> Self {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register_contract(None, FluxoraStream);
        let token_admin = Address::generate(&env);
        let token_id = env
            .register_stellar_asset_contract_v2(token_admin)
            .address();

        let admin = Address::generate(&env);
        let sender = Address::generate(&env);
        let recipient = Address::generate(&env);

        let client = FluxoraStreamClient::new(&env, &contract_id);
        client.init(&token_id, &admin);

        // Large mint so doc validation-table cases (10M+ raw units) can fund streams.
        let sac = StellarAssetClient::new(&env, &token_id);
        sac.mint(&sender, &100_000_000_i128);

        let token = TokenClient::new(&env, &token_id);
        token.approve(&sender, &contract_id, &i128::MAX, &100_000);

        TestContext {
            env,
            contract_id,
            sender,
            recipient,
            token,
        }
    }

    fn client(&self) -> FluxoraStreamClient<'_> {
        FluxoraStreamClient::new(&self.env, &self.contract_id)
    }

    /// Linear stream: rate=1 raw/s so timestamp == withdrawable (before any prior withdraw).
    fn create_linear_stream(&self, deposit: i128, threshold: i128, end_time: u64) -> u64 {
        self.client().create_stream(
            &self.sender,
            &CreateStreamParams {
                recipient: self.recipient.clone(),
                deposit_amount: deposit,
                rate_per_second: 1_i128,
                start_time: 0u64,
                cliff_time: 0u64,
                end_time: end_time,
                withdraw_dust_threshold: Some(threshold),
                memo: None,
                metadata: None,
                kind: StreamKind::Linear,
                irrevocable: None,
                witness: None,
            },
        )
    }
}

/// Expected outcome for a non-terminal, non-final-drain withdrawal.
#[derive(Clone, Copy, Debug)]
struct BoundaryCase {
    /// Label for assertion messages.
    name: &'static str,
    threshold: i128,
    /// Ledger timestamp (= withdrawable when rate=1 and no prior withdraw).
    withdrawable: i128,
    /// Doc-expected: allowed when withdrawable >= threshold.
    expect_allowed: bool,
}

/// Docs validation table + explicit ±1 boundaries around `threshold`.
///
/// From `docs/dust-threshold.md`:
/// - `withdrawable < threshold`  → blocked (return 0)
/// - `withdrawable == threshold` → allowed (strict `<`)
/// - `withdrawable > threshold`  → allowed
const BOUNDARY_CASES: &[BoundaryCase] = &[
    BoundaryCase {
        name: "threshold-1 (blocked)",
        threshold: 100,
        withdrawable: 99,
        expect_allowed: false,
    },
    BoundaryCase {
        name: "threshold (allowed, exact)",
        threshold: 100,
        withdrawable: 100,
        expect_allowed: true,
    },
    BoundaryCase {
        name: "threshold+1 (allowed)",
        threshold: 100,
        withdrawable: 101,
        expect_allowed: true,
    },
    // Doc validation-table rows (non-bypass).
    BoundaryCase {
        name: "doc: threshold=0, withdrawable=1 (allowed no-op threshold)",
        threshold: 0,
        withdrawable: 1,
        expect_allowed: true,
    },
    BoundaryCase {
        name: "doc: below threshold (blocked)",
        threshold: 10_000_000,
        withdrawable: 5_000_000,
        expect_allowed: false,
    },
    BoundaryCase {
        name: "doc: above threshold (allowed)",
        threshold: 10_000_000,
        withdrawable: 15_000_000,
        expect_allowed: true,
    },
    BoundaryCase {
        name: "doc: exactly at threshold (allowed)",
        threshold: 10_000_000,
        withdrawable: 10_000_000,
        expect_allowed: true,
    },
];

fn expected_amount(case: &BoundaryCase) -> i128 {
    if case.expect_allowed {
        case.withdrawable
    } else {
        0
    }
}

// ---------------------------------------------------------------------------
// Table-driven boundary tests — one per dust-consulting entry point
// ---------------------------------------------------------------------------

#[test]
fn table_withdraw_dust_boundaries_threshold_minus_exact_plus() {
    for case in BOUNDARY_CASES {
        let ctx = TestContext::setup();
        ctx.env.ledger().set_timestamp(0);

        // End far after withdrawable so we stay non-terminal / non-final-drain.
        let end_time = (case.withdrawable as u64).saturating_add(10_000);
        let deposit = case
            .withdrawable
            .saturating_add(10_000)
            .max(case.threshold + 1);
        let stream_id = ctx.create_linear_stream(deposit, case.threshold, end_time);

        ctx.env.ledger().set_timestamp(case.withdrawable as u64);
        let withdrawn = ctx.client().withdraw(&stream_id, &None);
        let want = expected_amount(case);
        assert_eq!(
            withdrawn, want,
            "withdraw {}: got {}, want {} (threshold={}, withdrawable={})",
            case.name, withdrawn, want, case.threshold, case.withdrawable
        );
        assert_eq!(
            ctx.token.balance(&ctx.recipient),
            want,
            "withdraw {}: recipient balance mismatch",
            case.name
        );
    }
}

#[test]
fn table_withdraw_to_dust_boundaries_threshold_minus_exact_plus() {
    for case in BOUNDARY_CASES {
        let ctx = TestContext::setup();
        ctx.env.ledger().set_timestamp(0);
        let destination = Address::generate(&ctx.env);

        let end_time = (case.withdrawable as u64).saturating_add(10_000);
        let deposit = case
            .withdrawable
            .saturating_add(10_000)
            .max(case.threshold + 1);
        let stream_id = ctx.create_linear_stream(deposit, case.threshold, end_time);

        ctx.env.ledger().set_timestamp(case.withdrawable as u64);
        let withdrawn = ctx.client().withdraw_to(&stream_id, &destination);
        let want = expected_amount(case);
        assert_eq!(
            withdrawn, want,
            "withdraw_to {}: got {}, want {}",
            case.name, withdrawn, want
        );
        assert_eq!(
            ctx.token.balance(&destination),
            want,
            "withdraw_to {}: destination balance mismatch",
            case.name
        );
        assert_eq!(
            ctx.token.balance(&ctx.recipient),
            0,
            "withdraw_to {}: recipient must not receive tokens",
            case.name
        );
    }
}

#[test]
fn table_batch_withdraw_dust_boundaries_threshold_minus_exact_plus() {
    for case in BOUNDARY_CASES {
        let ctx = TestContext::setup();
        ctx.env.ledger().set_timestamp(0);

        let end_time = (case.withdrawable as u64).saturating_add(10_000);
        let deposit = case
            .withdrawable
            .saturating_add(10_000)
            .max(case.threshold + 1);
        let stream_id = ctx.create_linear_stream(deposit, case.threshold, end_time);

        ctx.env.ledger().set_timestamp(case.withdrawable as u64);
        let results = ctx
            .client()
            .batch_withdraw(&ctx.recipient, &vec![&ctx.env, stream_id]);
        let want = expected_amount(case);
        assert_eq!(
            results.get(0).unwrap().amount,
            want,
            "batch_withdraw {}: got {}, want {}",
            case.name,
            results.get(0).unwrap().amount,
            want
        );
        assert_eq!(
            ctx.token.balance(&ctx.recipient),
            want,
            "batch_withdraw {}: recipient balance mismatch",
            case.name
        );
    }
}

#[test]
fn table_batch_withdraw_to_dust_boundaries_threshold_minus_exact_plus() {
    for case in BOUNDARY_CASES {
        let ctx = TestContext::setup();
        ctx.env.ledger().set_timestamp(0);
        let destination = Address::generate(&ctx.env);

        let end_time = (case.withdrawable as u64).saturating_add(10_000);
        let deposit = case
            .withdrawable
            .saturating_add(10_000)
            .max(case.threshold + 1);
        let stream_id = ctx.create_linear_stream(deposit, case.threshold, end_time);

        ctx.env.ledger().set_timestamp(case.withdrawable as u64);
        let param = WithdrawToParam {
            stream_id,
            destination: destination.clone(),
        };
        let results = ctx
            .client()
            .batch_withdraw_to(&ctx.recipient, &vec![&ctx.env, param]);
        let want = expected_amount(case);
        assert_eq!(
            results.get(0).unwrap().amount,
            want,
            "batch_withdraw_to {}: got {}, want {}",
            case.name,
            results.get(0).unwrap().amount,
            want
        );
        assert_eq!(
            ctx.token.balance(&destination),
            want,
            "batch_withdraw_to {}: destination balance mismatch",
            case.name
        );
    }
}

// ---------------------------------------------------------------------------
// Bypass conditions from docs (terminal / final drain)
// ---------------------------------------------------------------------------

#[test]
fn withdraw_dust_threshold_bypassed_on_final_drain() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let stream_id = ctx.create_linear_stream(1000, 500, 1000);

    ctx.env.ledger().set_timestamp(950);
    assert_eq!(ctx.client().withdraw(&stream_id, &None), 950);

    // Remaining 50 < threshold 500, but final drain (withdrawn + withdrawable == deposit).
    ctx.env.ledger().set_timestamp(1000);
    assert_eq!(
        ctx.client().withdraw(&stream_id, &None),
        50,
        "final drain must bypass dust threshold per docs"
    );
}

#[test]
fn withdraw_dust_threshold_bypassed_when_cancelled() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let stream_id = ctx.create_linear_stream(1000, 500, 1000);
    ctx.env.ledger().set_timestamp(100);
    ctx.client().cancel_stream(&stream_id);

    assert_eq!(
        ctx.client().withdraw(&stream_id, &None),
        100,
        "Cancelled terminal state must bypass dust threshold per docs"
    );
}

#[test]
fn withdraw_dust_threshold_bypassed_past_end_time() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let stream_id = ctx.create_linear_stream(1000, 500, 1000);
    ctx.env.ledger().set_timestamp(900);
    ctx.client().withdraw(&stream_id, &None);

    ctx.env.ledger().set_timestamp(1100);
    assert_eq!(
        ctx.client().withdraw(&stream_id, &None),
        100,
        "past end_time terminal bypass must allow remaining balance"
    );
}

// ---------------------------------------------------------------------------
// Doc / code mismatch flags (assert actual behavior; do not "fix" production)
// ---------------------------------------------------------------------------

/// Doc mismatch: `create_stream` currently accepts threshold > deposit (offer path rejects).
#[test]
fn flag_mismatch_create_allows_threshold_above_deposit() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let deposit = 1_000_i128;
    let oversized = deposit + 1;
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &CreateStreamParams {
            recipient: ctx.recipient.clone(),
            deposit_amount: deposit,
            rate_per_second: 1_i128,
            start_time: 0u64,
            cliff_time: 0u64,
            end_time: 1000u64,
            withdraw_dust_threshold: Some(oversized),
            memo: None,
            metadata: None,
            kind: StreamKind::Linear,
            irrevocable: None,
            witness: None,
        },
    );

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(
        state.withdraw_dust_threshold, oversized,
        "create_stream currently accepts threshold > deposit (docs say reject)"
    );
}

/// Doc mismatch: `create_stream` currently accepts negative thresholds (offer path rejects).
#[test]
fn flag_mismatch_create_allows_negative_dust_threshold() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &CreateStreamParams {
            recipient: ctx.recipient.clone(),
            deposit_amount: 1000_i128,
            rate_per_second: 1_i128,
            start_time: 0u64,
            cliff_time: 0u64,
            end_time: 1000u64,
            withdraw_dust_threshold: Some(-1_i128),
            memo: None,
            metadata: None,
            kind: StreamKind::Linear,
            irrevocable: None,
            witness: None,
        },
    );

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(
        state.withdraw_dust_threshold, -1,
        "create_stream currently accepts negative threshold (docs say reject)"
    );
}

/// `create_stream_offer` rejects threshold > deposit with InvalidDustThreshold (code 35).
#[test]
fn create_stream_offer_rejects_threshold_above_deposit() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let deposit = 1_000_i128;
    let result = ctx.client().try_create_stream_offer(
        &ctx.sender,
        &CreateStreamParams {
            recipient: ctx.recipient.clone(),
            deposit_amount: deposit,
            rate_per_second: 1_i128,
            start_time: 0u64,
            cliff_time: 0u64,
            end_time: 1000u64,
            withdraw_dust_threshold: Some(deposit + 1),
            memo: None,
            metadata: None,
            kind: StreamKind::Linear,
            irrevocable: None,
            witness: None,
        },
        &None,
    );

    assert_eq!(
        result,
        Err(Ok(fluxora_stream::ContractError::InvalidDustThreshold)),
        "create_stream_offer must reject threshold > deposit"
    );
}

/// `create_stream_offer` rejects negative threshold with InvalidDustThreshold (code 35).
#[test]
fn create_stream_offer_rejects_negative_dust_threshold() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let result = ctx.client().try_create_stream_offer(
        &ctx.sender,
        &CreateStreamParams {
            recipient: ctx.recipient.clone(),
            deposit_amount: 1000_i128,
            rate_per_second: 1_i128,
            start_time: 0u64,
            cliff_time: 0u64,
            end_time: 1000u64,
            withdraw_dust_threshold: Some(-1_i128),
            memo: None,
            metadata: None,
            kind: StreamKind::Linear,
            irrevocable: None,
            witness: None,
        },
        &None,
    );

    assert_eq!(
        result,
        Err(Ok(fluxora_stream::ContractError::InvalidDustThreshold)),
        "create_stream_offer must reject negative threshold"
    );
}

/// Boundary test: withdraw_dust_threshold == deposit_amount should succeed at creation.
#[test]
fn create_allows_threshold_equal_to_deposit() {
    let ctx = TestContext::setup();
    ctx.env.ledger().set_timestamp(0);

    let deposit = 1_000_i128;
    let threshold = deposit; // threshold == deposit is allowed (boundary case)
    let stream_id = ctx.client().create_stream(
        &ctx.sender,
        &CreateStreamParams {
            recipient: ctx.recipient.clone(),
            deposit_amount: deposit,
            rate_per_second: 1_i128,
            start_time: 0u64,
            cliff_time: 0u64,
            end_time: 1000u64,
            withdraw_dust_threshold: Some(threshold),
            memo: None,
            metadata: None,
            kind: StreamKind::Linear,
            irrevocable: None,
            witness: None,
        },
    );

    let state = ctx.client().get_stream_state(&stream_id);
    assert_eq!(
        state.withdraw_dust_threshold, threshold,
        "threshold == deposit should be accepted at creation"
    );
}
