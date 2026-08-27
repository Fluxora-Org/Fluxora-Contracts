//! Test suite, staged to match the build order.
//!
//! * **Stage 1** — data model, create, withdraw, views, plus the two tests that
//!   gate everything else: the accrual property suite and the pool invariant.
//! * **Stage 2** — cliff, cancel, pause/resume, top-up, recipient transfer, and
//!   every adversarial boundary case.
//! * **Stage 3** — TTL survival and archival recovery, resource consumption at
//!   the batch cap.

mod common;
mod missing;

// Stage 1
mod create;
mod props;
mod withdraw;

// Stage 2
mod auth;
mod cancel;
// Issue #1584: the cancellation event's accounting contract.
mod amount_domain;
mod cancel_events;
mod cliff;
mod delegation;
mod pause;
mod storage_keys;
mod token_errors;
mod top_up;
mod transfer;

// Stage 3
mod accrual_overflow;
mod batch;
mod invariants;
mod monotonicity;
mod resource_limits;
mod ttl;

// Issue #1593 — reproducible ledger and token state on failure
mod snapshot_tests;

// Issue #1595 — release dry-run pre-flight validation
mod release_dry_run;
