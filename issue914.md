Description
contracts/governance/tests/signer_index_proptest.rs already exists with an associated .proptest-regressions file, indicating it has found and pinned failing cases before. Review the current regression corpus, ensure every pinned regression case has a corresponding explicit (non-random) unit test so it can't silently regress if the proptest seed/config changes, and document the invariant being tested in docs/governance.md.

Requirements
Read the .proptest-regressions file and convert each pinned case into an explicit unit test with a comment explaining what it caught.
Document the signer-index invariant (what makes an index valid/invalid) in docs/governance.md.
Confirm the proptest itself still runs with a reasonable case count in normal CI (not just on-demand).
Suggested execution
Review signer_index_proptest.rs and its regressions file.
Add explicit unit tests for each pinned regression case.
Add the governance doc section describing the invariant.
Acceptance criteria

Every pinned proptest regression has an explicit unit test.

Invariant is documented in docs/governance.md.

Proptest continues to run as part of normal test execution.
Security notes
Signer-index handling is core to multisig authorization correctness; pinning known-bad cases as explicit tests prevents silent re-introduction of a previously-fixed bug.

Guidelines
Minimum 95% test coverage
