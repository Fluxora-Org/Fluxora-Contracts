Description
contracts/factory/tests/factory_init_security.rs, batch_pause_gate.rs, and adversarial_auth.rs already cover parts of this, but add a combined matrix test covering every mutating entry point crossed with every contract state (uninitialized, initialized-active, paused) to confirm each entry point's behavior in each state is intentional and documented, not just individually spot-checked.

Requirements
Enumerate every public mutating function in factory/src/lib.rs.
For each, assert its behavior (succeed / typed-reject) in each of the three states.
Where a function unexpectedly succeeds pre-init or while paused, flag it explicitly for maintainer review rather than assuming it's a bug.
Suggested execution
List all public mutating functions and cross-reference existing tests in factory_init_security.rs/batch_pause_gate.rs for coverage gaps.
Fill in the matrix with new test cases for any uncovered (function, state) pair.
Summarize findings (especially any surprising allowed combination) in the PR description.
Acceptance criteria

Full (function x state) matrix is tested, building on existing coverage rather than duplicating it.

Any surprising/unintended allowed combination is flagged for review.

Tests pass under cargo test -p fluxora_factory (or equivalent package name).
Security notes
State-machine gaps (an action allowed in a state it shouldn't be) are a classic source of smart-contract vulnerabilities; this is a systematic sweep for exactly that class of bug.

Guidelines
Minimum 95% test coverage
Timeframe: 96 hours
