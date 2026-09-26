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
mod events;
mod missing;

// ABI inventory — generated from the contract spec, independent of stage.
mod abi;

// Issue #1535 — discriminant fixture and public error-path regression tests.
mod error_discriminants;

// Stage 1
mod create;
mod props;
mod withdraw;
// Issue #1583: withdrawal return value matches emitted amounts.
mod withdraw_events;

// Stage 2
mod auth;
mod cancel;
// Issue #1726: capability flags are set at creation and immutable.
mod capabilities;
// Issue #1584: the cancellation event's accounting contract.
mod amount_domain;
mod cancel_events;
mod cliff;
mod delegation;
mod pause;
mod storage_keys;
mod terminal_operations;
mod token_errors;
mod top_up;
mod transfer;

// Stage 3
mod accounting_identity;
mod accrual_overflow;
mod batch;
mod entrypoint_costs;
mod invariants;
mod lifecycle_proptest;
mod monotonicity;
mod release_profile;
mod resource_limits;
mod ttl;

// Stage 4
mod stream_ids;

// Issue #1686: every read entry point's storage/TTL behaviour, pinned to
// docs/ABI.md. `read_methods_no_side_effects` (#1566) existed but was never
// registered here, so it did not compile or run until now.
mod read_methods_no_side_effects;
mod read_ttl_matrix;

// Package / artifact naming gates, run by CI's `packaging::` step. Also
// guards #1675 (no inert governance crate). Previously unregistered, so
// that CI step matched zero tests.
mod packaging;
