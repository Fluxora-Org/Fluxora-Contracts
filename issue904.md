Description
contracts/stream/src/lib.rs recently gained cumulative_paused_duration tracking on Stream, incremented in resume_stream using checked_add. Add explicit boundary tests that push this field close to u64::MAX (via repeated pause/resume cycles or a crafted ledger timestamp jump) to confirm the checked_add path actually triggers its overflow error rather than being unreachable in practice.

Requirements
Add a test harness that can drive cumulative_paused_duration near u64::MAX without requiring an impractical number of real pause/resume cycles (e.g. by manipulating ledger timestamp deltas directly in test setup).
Assert the contract returns a typed error (not a panic) when the checked-add would overflow.
Confirm get_paused_duration and all calculate_accrued_amount_checkpointed call sites listed in the recent PR remain consistent when this field is at its boundary.
Suggested execution
Review resume_stream and get_paused_duration in contracts/stream/src/lib.rs.
Add a boundary test in contracts/stream/tests/ (or src/test.rs) using a large synthetic pause duration.
Run cargo test -p fluxora_stream and record output in the PR description.
Acceptance criteria

Overflow path returns a typed error, not a panic.

Boundary test is deterministic and doesn't require excessive real-time simulation.

All checkpoint call sites remain consistent at the boundary.
Security notes
An unreachable-in-practice checked-add is fine, but an actually-reachable panic on this field would be a denial-of-service on that stream's read/write paths.

Guidelines
Minimum 95% test coverage
Timeframe: 96 hours