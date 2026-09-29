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

// Issue #1689 — every discriminant is produced by a public entry point or
// listed in the frozen reserved allowlist.
mod error_reachability;

// Issue #1879 — randomized operation-sequence search proving `VestedDecreased`
// (33) is unreachable, and documenting it as a defensive invariant.
mod vested_decreased;

// Stage 1
mod create;
mod props;
mod reference;
mod withdraw;
// Issue #1583: withdrawal return value matches emitted amounts.
mod withdraw_events;
// Issue #1839: withdrawing exactly the full withdrawable amount — the boundary
// where `withdrawable` reaches zero and the follow-up error changes from
// `NothingToWithdraw` to `StreamTerminated`.
mod withdraw_exact_balance;

// Stage 2
mod auth;
mod cancel;
// Issue #1726: capability flags are set at creation and immutable.
mod capabilities;
// Issue #1584: the cancellation event's accounting contract.
mod amount_domain;
mod cancel_events;
mod cliff;
// Wall-clock vs schedule-relative cliff gate. `test::cliff` pins the
// schedule-relative half (issue #1688); this is the opt-out that pausing
// cannot move. See `docs/KNOWN-LIMITATIONS.md` §7.
mod cliff_mode;
mod delegation;
// Issue #1838 — a delegate acting in the very ledger its grant expires.
mod delegate_expiry_boundary;
// Issue #1734: comprehensive revoke_delegate coverage — per-bit, no-op on
// never-issued grants, same-ledger effect, and multi-delegate isolation.
mod revoke_delegate;
// Issue #1854: two delegates holding WITHDRAW on one stream settle in the
// same ledger serialised by storage — no double settlement, funds conserved.
mod delegate_concurrent_withdraw;
mod pause;
mod storage_keys;
mod terminal_operations;
mod token_errors;

// Issue #1883 — withdrawal from a stream whose token contract is gone.
mod token_destroyed;
mod top_up;
mod transfer;
mod withdraw_cancel_same_ledger;

// Stage 3
mod accounting_identity;
// Issue #1856 — the `withdrawable + refundable == deposited - withdrawn`
// identity as a generated property over randomized operation sequences.
mod accounting_property;
mod accrual_overflow;
mod batch;
// Issue #1811: bounded batch cancellation, reported by index on refusal.
mod batch_cancel;

// Issue #1866 — the MAX_BATCH_SIZE ceiling across every batch entry point.
mod batch_ceiling;
mod entrypoint_costs;
mod invariants;
mod lifecycle_proptest;
mod monotonicity;
mod release_profile;
mod resource_limits;
mod ttl;

// Stage 4
mod stream_ids;

// Issue #1870 — the documented migration path, walked and cross-checked
// against the committed ABI inventory.
mod migration;

// Invariant: no success event emitted on a reverting token transfer (#1728).
mod event_ordering_failed_transfer;

// Issue #1699 — `stream_count()` vs. the population of stream records,
// asserted after failed creations, after every terminal operation, under
// deliberate counter corruption, and across randomized sequences.
mod stream_count_consistency;

// Issue #1686: every read entry point's storage/TTL behaviour, pinned to
// docs/ABI.md. `read_methods_no_side_effects` (#1566) existed but was never
// registered here, so it did not compile or run until now.
mod read_methods_no_side_effects;
mod read_ttl_matrix;

// Issue #1850 — an id at or beyond `stream_count()` was never issued, and is
// distinguishable from an archived one.
mod stream_exists_bounds;

// Package / artifact naming gates, run by CI's `packaging::` step. Also
// guards #1675 (no inert governance crate). Previously unregistered, so
// that CI step matched zero tests.
mod packaging;

// Issue #1840 — a stream funded with the maximum representable deposit
// (`i128::MAX`), driven end to end through the public ABI on a dedicated
// full-range asset; also pins the creation-guard boundary that rejects it.
mod max_deposit;
