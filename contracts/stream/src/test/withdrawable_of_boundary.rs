//! Boundary tests for [`FluxoraStream::withdrawable_of`] — issue #2071.
//!
//! `withdrawable_of(stream_id)` accepts any id in `0..stream_count()`.
//! Ids outside that range have no backing storage record and return
//! [`Error::StreamNotFound`].
//!
//! | boundary | stream_id | expected |
//! |---|---|---|
//! | zero (first valid id) | 0 | `Ok(≥ 0)` |
//! | one (second valid id) | 1 | `Ok(≥ 0)` |
//! | max accepted (`stream_count() - 1`) | N-1 | `Ok(≥ 0)` |
//! | one past max (`stream_count()`) | N | `Err(StreamNotFound)` |
//!
//! Every refusal is a typed [`Error::StreamNotFound`] — no test panics.

use super::common::*;
use crate::Error;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Create N identical streams and return their ids.
fn create_n(h: &Harness, n: u64) -> std::vec::Vec<u64> {
    (0..n).map(|_| h.create_simple(100 * ONE, DAY)).collect()
}

// ---------------------------------------------------------------------------
// stream_id = 0 — first valid id
// ---------------------------------------------------------------------------

/// stream_id 0 is the first valid id. With one stream created it must succeed.
#[test]
fn stream_id_zero_is_accepted() {
    let h = Harness::new();
    create_n(&h, 1);

    let result = h.client.try_withdrawable_of(&0_u64);
    assert!(
        result.is_ok(),
        "stream_id=0 (first valid id) must return Ok, got {result:?}"
    );
}

/// stream_id 0 returns a non-negative amount — not a panic, not an error.
#[test]
fn stream_id_zero_returns_non_negative_amount() {
    let h = Harness::new();
    create_n(&h, 1);

    let amount = h.client.withdrawable_of(&0_u64);
    assert!(
        amount >= 0,
        "withdrawable_of(0) must be >= 0, got {amount}"
    );
}

// ---------------------------------------------------------------------------
// stream_id = 1 — one (second valid id)
// ---------------------------------------------------------------------------

/// stream_id 1 is the second valid id. With two streams created it must succeed.
#[test]
fn stream_id_one_is_accepted() {
    let h = Harness::new();
    create_n(&h, 2);

    let result = h.client.try_withdrawable_of(&1_u64);
    assert!(
        result.is_ok(),
        "stream_id=1 (second valid id) must return Ok, got {result:?}"
    );
}

/// stream_id 1 also returns a non-negative amount.
#[test]
fn stream_id_one_returns_non_negative_amount() {
    let h = Harness::new();
    create_n(&h, 2);

    let amount = h.client.withdrawable_of(&1_u64);
    assert!(
        amount >= 0,
        "withdrawable_of(1) must be >= 0, got {amount}"
    );
}

// ---------------------------------------------------------------------------
// stream_id = N-1 — maximum accepted value (last valid id)
// ---------------------------------------------------------------------------

/// The last valid id (`stream_count() - 1`) must be accepted.
#[test]
fn stream_id_max_accepted_value_is_ok() {
    let h = Harness::new();
    let n: u64 = 3;
    create_n(&h, n);

    let max_id = n - 1;
    let result = h.client.try_withdrawable_of(&max_id);
    assert!(
        result.is_ok(),
        "stream_id=stream_count()-1 (max accepted) must return Ok, got {result:?}"
    );
}

/// The last valid id returns a non-negative amount.
#[test]
fn stream_id_max_accepted_returns_non_negative_amount() {
    let h = Harness::new();
    let n: u64 = 3;
    create_n(&h, n);

    let max_id = n - 1;
    let amount = h.client.withdrawable_of(&max_id);
    assert!(
        amount >= 0,
        "withdrawable_of({max_id}) must be >= 0, got {amount}"
    );
}

// ---------------------------------------------------------------------------
// stream_id = N — one past the maximum (never issued)
// ---------------------------------------------------------------------------

/// `stream_count()` itself has never been issued. Must return StreamNotFound,
/// not panic.
#[test]
fn stream_id_one_past_max_is_stream_not_found() {
    let h = Harness::new();
    let n: u64 = 3;
    create_n(&h, n);

    // N is exactly one past the last valid id.
    let one_past = n;
    let result = h.client.try_withdrawable_of(&one_past);
    assert_eq!(
        result,
        Err(Ok(Error::StreamNotFound)),
        "stream_id=stream_count() (one past max) must return StreamNotFound, not panic"
    );
}

/// Zero streams created: stream_id 0 is already one past the max.
#[test]
fn stream_id_zero_on_empty_contract_is_stream_not_found() {
    let h = Harness::new();
    // No streams created — stream_count() == 0, so id 0 is beyond the range.

    let result = h.client.try_withdrawable_of(&0_u64);
    assert_eq!(
        result,
        Err(Ok(Error::StreamNotFound)),
        "stream_id=0 with no streams created must return StreamNotFound, not panic"
    );
}

/// A large id that was never issued must also return StreamNotFound, not panic.
#[test]
fn stream_id_large_never_issued_is_stream_not_found() {
    let h = Harness::new();
    create_n(&h, 1);

    let large_id: u64 = u64::MAX;
    let result = h.client.try_withdrawable_of(&large_id);
    assert_eq!(
        result,
        Err(Ok(Error::StreamNotFound)),
        "stream_id=u64::MAX (never issued) must return StreamNotFound, not panic"
    );
}
