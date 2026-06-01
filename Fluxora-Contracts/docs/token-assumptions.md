# Token assumptions and SEP-41 conformance

This document records the token behaviour the `soroban-stream` contract expects and
describes the test matrix used to validate those assumptions.

Summary
- The stream contract assumes the token at `Config.token` implements the SEP-41
  / Soroban Asset Contract (SAC) entrypoints used by the contract: `transfer`,
  `transfer_from`, `approve`, `balance` and (test-only) `mint`.
- It assumes token transfers either succeed or fail explicitly (no silent success
  indicators), and that token contracts do not re-enter the streaming contract
  during transfer callbacks.

Threat model & Security notes
- Reentrancy: the contract follows CEI ordering (persist state before external
  token calls) but cannot fully prevent a malicious token from re-entering. Use
  well-audited tokens in production.
- Silent failures: tokens that return `false` from `transfer`/`transfer_from` or
  otherwise deviate from the expected interface can cause unexpected behaviour.
  The contract treats such deviations as failures in testing and will rollback.
- Approval revocation: callers may revoke approvals after granting them; callers
  must ensure approvals exist at call time for operations that pull funds.

Test matrix
- The test `contracts/stream/tests/sep41_matrix.rs` exercises the following
  implementations against these entrypoints: `create_stream`, `withdraw`,
  `batch_withdraw`, `top_up_stream`, and `cancel_stream`.
- Token behaviours covered by the tests:
  - Normal token (StellarAsset V2): expected to succeed for all flows.
  - Approve-then-revoke token: enforces allowance checks and models allowance
    revocation between operations.
  - Transfer-returning-false token: simulates tokens that return `false` from
    transfer functions.
  - Panic-on-transfer token: panics during transfers to simulate a misbehaving contract.

How to run

Run the stream package tests and the matrix specifically:

```bash
cargo test -p soroban-stream --tests
cargo test -p soroban-stream --test sep41_matrix
```

Notes
- These tests are deliberately strict: they assert success for well-behaved
  tokens and expect failures (panics or errors) for misbehaving tokens. They
  document the contract's tolerance and expose areas where integrators must
  validate token behaviour before initializing the stream contract.
