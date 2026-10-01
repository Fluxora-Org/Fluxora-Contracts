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

// Issue #1818 — contract-level emergency halt: one-shot operator install,
// halt/resume, and the "every mutating entry point is refused while reads
// still answer" acceptance test.
mod halt;
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
mod settled_dust;
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
// Issue #1824: `delegate_top_up` held to the rejection depth of `top_up`.
mod delegate_top_up;
mod delegation;
// Issue #1845: delegation surviving a recipient transfer.
mod delegation_transfer;

// Issue #1882 — stream parties on the delegate paths without a grant.
mod delegate_party_without_grant;
// Issue #1881 — two delegates sharing one permission on one stream.
mod multi_delegate;
// Issue #1838 — a delegate acting in the very ledger its grant expires.
mod delegate_expiry_boundary;
// Issue #1734: comprehensive revoke_delegate coverage — per-bit, no-op on
// never-issued grants, same-ledger effect, and multi-delegate isolation.
mod revoke_delegate;
// Issue #1880 — a delegate grant covering several permission bits permits
// exactly those ops; revoking one bit leaves the others intact; clearing every
// bit is equivalent to revoking; an ungrated op is rejected even when others
// are present.
mod delegate_multi_bit_grant;
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

// Issue #1805 — a rebasing token that changes the pool's balance outside a
// transfer is detected at the next operation that moves funds
// (`Error::PoolBalanceDrift`) instead of silently desynchronising the pool.
mod rebase_drift;
// Issue #1815 — non-linear release curves: monotonicity, total conservation,
// and backwards compatibility of the frozen v1 storage layout.
mod release_curves;

// Stage 3
mod accounting_identity;
// Issue #1856 — the `withdrawable + refundable == deposited - withdrawn`
// identity as a generated property over randomized operation sequences.
mod accounting_property;
mod accrual_overflow;
mod batch;
// Issue #1810 — atomic, bounded payroll-style stream creation.
mod batch_create;
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

// Issue #1875 — docs/ARCHITECTURE.md, checked against the code it describes.
mod architecture;
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

// Issue #1828 — `delegate_withdraw` coverage at parity with the direct path.
mod delegate_withdraw;
// Issue #1804 — `MAX_BATCH_SIZE` calibrated against more than one token
// implementation.
mod token_batch_calibration;
// Issue #1835 — a recipient transfer in the same ledger as a withdrawal.
mod transfer_withdraw_same_ledger;
// Issue — top_up changes deposited/end_time; withdraw reads vested from the
// same storage. Both orderings in the same ledger are covered and shown to
// be conservation-equivalent (rate-preserving property of top_up).
mod top_up_withdraw_same_ledger;
// Issue #1857 — the contract's token balance always covers the summed live
// stream liability, asserted over randomized operation sequences.
mod pool_liability_proptest;
// Issue #1858 — no sequence of operations lets total payouts exceed the deposit.
mod payout_conservation;
// Issue #1852 — TTL extension on a stream at its minimum TTL floor.
mod ttl_minimum_extension;
// Issue #1850 — an id at or beyond `stream_count()` was never issued, and is
// distinguishable from an archived one.
mod stream_exists_bounds;

// Package / artifact naming gates, run by CI's `packaging::` step. Also
// guards #1675 (no inert governance crate). Previously unregistered, so
// that CI step matched zero tests.
mod packaging;

// Issue #1868 — replaying the event stream alone must reconstruct every
// stream's state, so an indexer with no on-chain per-party index can answer
// "which streams are mine" and keep its mirror of `get_stream` correct.
mod event_reconstruction;
// Issue #1860 — the id allocator must be a function of the counter alone, never of which records happen to be present
mod id_reuse_proptest;
// Issue #1842 — treat `paused_total` as an accumulator — including the events that carry it and the `u64` bound it grows against
mod paused_total_cycles;
// Issue #1851 — `cancel` settles the record in place — rewriting `deposited` and collapsing `end_time` — so `get_stream` is the entry point that has to report a moved schedule
mod get_stream_cancelled;
// Issue #1841 — a one-second schedule is the smallest non-degenerate stream, and the only duration where the vesting curve is two points and the dust-rate floor is unreachable
mod one_second_stream;
// Issue #1840 — a stream funded with the maximum representable deposit
// (`i128::MAX`), driven end to end through the public ABI on a dedicated
// full-range asset; also pins the creation-guard boundary that rejects it.
mod max_deposit;
