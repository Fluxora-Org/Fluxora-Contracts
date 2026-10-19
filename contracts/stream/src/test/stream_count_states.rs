//! `stream_count()` remains a readable, stable total across stream states.

use super::common::*;
use crate::{Error, StreamStatus};

#[test]
fn stream_count_returns_one_for_an_active_stream() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);

    assert_eq!(h.get(id).status, StreamStatus::Active);
    assert_eq!(h.client.stream_count(), 1);
}

#[test]
fn stream_count_returns_one_for_a_paused_stream() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.client.pause(&id);

    assert_eq!(h.get(id).status, StreamStatus::Paused);
    assert_eq!(h.client.stream_count(), 1);
}

#[test]
fn stream_count_stays_one_when_a_cancelled_stream_rejects_a_call() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.advance(30 * DAY);
    h.client.cancel(&id);

    assert_eq!(h.get(id).status, StreamStatus::Cancelled);
    assert_eq!(h.client.stream_count(), 1);

    let before = h.get(id);
    let err = h.client.try_cancel(&id).unwrap_err().unwrap();
    assert_eq!(err, Error::StreamTerminated);
    assert_eq!(
        h.get(id),
        before,
        "refused cancellation must not mutate the stream"
    );
    assert_eq!(h.client.stream_count(), 1);
}

#[test]
fn stream_count_stays_one_when_top_up_is_refused_after_maturity() {
    let h = Harness::new();
    let id = h.create_simple(1_000 * ONE, 100 * DAY);
    h.warp_to(T0 + 100 * DAY);

    // Maturity is determined by the schedule; it does not replace Active status.
    assert_eq!(h.get(id).status, StreamStatus::Active);
    assert_eq!(h.client.stream_count(), 1);

    let before = h.get(id);
    let err = h.client.try_top_up(&id, &(100 * ONE)).unwrap_err().unwrap();
    assert_eq!(err, Error::StreamMatured);
    assert_eq!(
        h.get(id),
        before,
        "refused top-up must not mutate the stream"
    );
    assert_eq!(h.client.stream_count(), 1);
}
