//! Boundary tests for [`FluxoraFactory::init`] — issue #2067.
//!
//! `init` accepts two numeric policy arguments whose extremes were previously
//! uncovered:
//!
//! * `max_deposit: i128` — must be `>= MIN_DEPOSIT_CAP` (1); zero and
//!   negative values return [`FactoryError::InvalidCap`].
//! * `min_duration: u64` — must be `<= MAX_MIN_DURATION_SECONDS`
//!   (100 × 365 × 24 × 60 × 60 = 3 153 600 000 s); one past the ceiling
//!   returns [`FactoryError::InvalidMinDuration`].
//!
//! Every refusal is a **typed error** — no test panics.

use fluxora_factory::{FactoryError, FluxoraFactory, FluxoraFactoryClient, MAX_MIN_DURATION_SECONDS, MIN_DEPOSIT_CAP};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Register the contract and return (env, contract_id, admin, stream_contract).
fn setup() -> (Env, Address, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    let cid = env.register(FluxoraFactory, ());
    let admin = Address::generate(&env);
    let stream = Address::generate(&env);
    (env, cid, admin, stream)
}

fn client(env: &Env, cid: &Address) -> FluxoraFactoryClient {
    FluxoraFactoryClient::new(env, cid)
}

// ---------------------------------------------------------------------------
// max_deposit boundary — MIN_DEPOSIT_CAP = 1
// ---------------------------------------------------------------------------

/// Zero is below MIN_DEPOSIT_CAP and must be refused with InvalidCap.
#[test]
fn max_deposit_zero_is_invalid_cap() {
    let (env, cid, admin, stream) = setup();
    let result = client(&env, &cid).try_init(&admin, &stream, &0_i128, &0_u64);
    assert_eq!(
        result,
        Err(Ok(FactoryError::InvalidCap)),
        "max_deposit=0 must return InvalidCap, not panic"
    );
}

/// One equals MIN_DEPOSIT_CAP exactly and must be accepted.
#[test]
fn max_deposit_one_is_valid() {
    let (env, cid, admin, stream) = setup();
    let result = client(&env, &cid).try_init(&admin, &stream, &1_i128, &0_u64);
    assert_eq!(
        result,
        Ok(()),
        "max_deposit=MIN_DEPOSIT_CAP (1) must be accepted"
    );
}

/// i128::MAX is the largest representable cap and must be accepted.
#[test]
fn max_deposit_i128_max_is_valid() {
    let (env, cid, admin, stream) = setup();
    let result = client(&env, &cid).try_init(&admin, &stream, &i128::MAX, &0_u64);
    assert_eq!(
        result,
        Ok(()),
        "max_deposit=i128::MAX must be accepted"
    );
}

/// Negative one is below MIN_DEPOSIT_CAP and must be refused with InvalidCap.
#[test]
fn max_deposit_negative_is_invalid_cap() {
    let (env, cid, admin, stream) = setup();
    let result = client(&env, &cid).try_init(&admin, &stream, &(-1_i128), &0_u64);
    assert_eq!(
        result,
        Err(Ok(FactoryError::InvalidCap)),
        "max_deposit=-1 must return InvalidCap, not panic"
    );
}

// ---------------------------------------------------------------------------
// min_duration boundary — MAX_MIN_DURATION_SECONDS = 3_153_600_000
// ---------------------------------------------------------------------------

/// Zero is the smallest possible duration and must be accepted.
#[test]
fn min_duration_zero_is_valid() {
    let (env, cid, admin, stream) = setup();
    let result = client(&env, &cid).try_init(&admin, &stream, &MIN_DEPOSIT_CAP, &0_u64);
    assert_eq!(
        result,
        Ok(()),
        "min_duration=0 must be accepted"
    );
}

/// One is above zero and well within the ceiling — must be accepted.
#[test]
fn min_duration_one_is_valid() {
    let (env, cid, admin, stream) = setup();
    let result = client(&env, &cid).try_init(&admin, &stream, &MIN_DEPOSIT_CAP, &1_u64);
    assert_eq!(
        result,
        Ok(()),
        "min_duration=1 must be accepted"
    );
}

/// MAX_MIN_DURATION_SECONDS itself sits on the inclusive ceiling and must be accepted.
#[test]
fn min_duration_max_accepted_value_is_valid() {
    let (env, cid, admin, stream) = setup();
    let result = client(&env, &cid).try_init(
        &admin,
        &stream,
        &MIN_DEPOSIT_CAP,
        &MAX_MIN_DURATION_SECONDS,
    );
    assert_eq!(
        result,
        Ok(()),
        "min_duration=MAX_MIN_DURATION_SECONDS must be accepted"
    );
}

/// One past the ceiling must be refused with InvalidMinDuration.
#[test]
fn min_duration_one_past_max_is_invalid() {
    let (env, cid, admin, stream) = setup();
    let one_past = MAX_MIN_DURATION_SECONDS + 1;
    let result = client(&env, &cid).try_init(
        &admin,
        &stream,
        &MIN_DEPOSIT_CAP,
        &one_past,
    );
    assert_eq!(
        result,
        Err(Ok(FactoryError::InvalidMinDuration)),
        "min_duration=MAX_MIN_DURATION_SECONDS+1 must return InvalidMinDuration, not panic"
    );
}

/// u64::MAX is far past the ceiling and must also be refused with InvalidMinDuration.
#[test]
fn min_duration_u64_max_is_invalid() {
    let (env, cid, admin, stream) = setup();
    let result = client(&env, &cid).try_init(
        &admin,
        &stream,
        &MIN_DEPOSIT_CAP,
        &u64::MAX,
    );
    assert_eq!(
        result,
        Err(Ok(FactoryError::InvalidMinDuration)),
        "min_duration=u64::MAX must return InvalidMinDuration, not panic"
    );
}
