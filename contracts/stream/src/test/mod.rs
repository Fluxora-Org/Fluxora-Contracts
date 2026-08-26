//! Test suite, staged to match the build order.
//!
//! * **Stage 1** — data model, create, withdraw, views, plus the two tests that
//!   gate everything else: the accrual property suite and the pool invariant.
//! * **Stage 2** — cliff, cancel, pause/resume, top-up, recipient transfer, and
//!   every adversarial boundary case.
//! * **Stage 3** — TTL survival and archival recovery, resource consumption at
//!   the batch cap.
//! * **Stage 4** — the stream id invariant: unique, strictly monotonic, and
//!   never consumed or reused by a failed create, independent of fixture
//!   order.

mod common;

// Stage 1
mod create;
mod props;
mod withdraw;

// Stage 2
mod auth;
mod cancel;
mod cliff;
mod pause;
mod top_up;
mod transfer;

// Stage 3
mod batch;
mod invariants;
mod monotonicity;
mod resource_limits;
mod ttl;

// Stage 4
mod stream_ids;
