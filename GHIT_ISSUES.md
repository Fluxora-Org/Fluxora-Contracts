---
type: Feature
title: FluxoraStream: one-shot init and immutable config bootstrap
labels: contracts, soroban, init, security
assignees: ''
---

## Description

### Summary

Within the Fluxora streaming contract, this work tightens externally visible assurances for one focused concern. Treasury operators, recipient-facing applications, and third-party auditors must be able to reason about this area using only on-chain observables and published protocol documentation—without inferring hidden rules from how the implementation happens to be structured. The scenarios captured by this issue title must therefore have crisp success semantics, crisp failure semantics, and no silent drift between stored state, user-visible errors, and emitted events. Authorization boundaries for this slice must be explicit: who may trigger each path, and what proof they must supply. In practical terms, treat “FluxoraStream: one-shot init and immutable config bootstrap” as the scope boundary: everything materially related to that caption should be covered by evidence (automated or documented) in this change, and anything intentionally excluded should be called out with rationale and residual risk.

**Issue caption:** FluxoraStream: one-shot init and immutable config bootstrap

### Domain context

This repository hosts the Fluxora streaming contract on Soroban (Stellar). Streams lock funded assets in contract
storage, release entitlement over time according to schedule parameters (including cliffs), and expose explicit roles:
senders who fund and may adjust certain parameters, recipients who claim vested value, administrators who may intervene
under policy, and—where applicable—permissionless callers for maintenance reads or cleanup. Integrators (wallets,
indexers, treasury tooling) depend on coherent state transitions, emitted signals, and error behavior when inputs or
timing are invalid.

### Work to complete

1. Characterize the **intended protocol semantics** for *FluxoraStream: one-shot init and immutable config bootstrap* in both success and failure cases, treating the **Summary** above as the authoritative scope statement.
2. Map which roles may participate, what each role must prove, and which operations must be impossible for non-authorized actors.
3. Enumerate edge cases driven by time (start, cliff, end, cancellation freeze), numeric ranges, and stream status combinations; capture expectations with tests or explicit audit exceptions.
4. Ensure externally visible behavior—persistent fields, contract errors, and emitted payloads—does not contradict documentation that integrators rely on.

### Definition of done

- An independent reader can confirm *FluxoraStream: one-shot init and immutable config bootstrap* matches the stated intent using tests, traces, snapshots (where used), or written review notes.
- Documentation for this slice of the protocol is consistent with observable on-chain behavior after the change.
- The pull request explains verification performed, scope boundaries, and any residual assumptions or risks.

### Constraints for contributors

- Describe **outcomes**, **invariants**, and **evidence**, not a single “right” internal design unless the issue title already names a concrete subsystem.
- Prefer **observable** guarantees: state transitions, balances, authorization failures, emitted events, error classifications, and documentation that integrators rely on.
- If something cannot be tested automatically, capture the gap as **audit notes** with explicit rationale and residual risk.

## Requirements and context

- Must be **secure**, **well-tested**, and **documented**.
- Should be **gas-conscious** (Soroban budget) and **easy to audit**.

## Suggested execution

1. Fork the repo and create a branch:
   ```bash
   git checkout -b feature/stream-init-bootstrap
   ```
2. Implement changes
   - **Contract / modules:** `contracts/stream/src/lib.rs`
   - **Comprehensive tests:** `contracts/stream/src/test.rs` and `contracts/stream/tests/integration_suite.rs`
   - **Documentation:** `docs/` (e.g. `streaming.md`, `security.md`, `events.md`)
   - Include **Rust doc comments** on public items (NatSpec-style clarity for auditors)
   - **Validate security assumptions** (CEI ordering, token trust model, auth paths)
3. **Test and commit**
   - Run `cargo test` in `contracts/stream` (and workspace if applicable)
   - Cover **edge cases** (overflow, TTL, paused/cancelled/completed)
   - Attach **test output** summary + **security notes** in the PR

### Example commit message

```
feat(stream): harden init and config bootstrap
```

## Guidelines

- Aim for **≥95%** coverage on touched Rust modules (`cargo llvm-cov` or project-standard tooling).
- **Clear documentation** (protocol semantics + operator runbooks where relevant)
- **Timeframe:** 96 hours

++++++

---
type: Feature
title: Regression tests: double-init and missing-config reads
labels: contracts, soroban, testing
assignees: ''
---

## Description

### Summary

Within the Fluxora streaming contract, this work tightens externally visible assurances for one focused concern. Treasury operators, recipient-facing applications, and third-party auditors must be able to reason about this area using only on-chain observables and published protocol documentation—without inferring hidden rules from how the implementation happens to be structured. The scenarios captured by this issue title must therefore have crisp success semantics, crisp failure semantics, and no silent drift between stored state, user-visible errors, and emitted events. Permissionless maintenance or read paths, if involved, must have clearly documented trust impact for operators. In practical terms, treat “Regression tests: double-init and missing-config reads” as the scope boundary: everything materially related to that caption should be covered by evidence (automated or documented) in this change, and anything intentionally excluded should be called out with rationale and residual risk.

**Issue caption:** Regression tests: double-init and missing-config reads

### Domain context

This repository hosts the Fluxora streaming contract on Soroban (Stellar). Streams lock funded assets in contract
storage, release entitlement over time according to schedule parameters (including cliffs), and expose explicit roles:
senders who fund and may adjust certain parameters, recipients who claim vested value, administrators who may intervene
under policy, and—where applicable—permissionless callers for maintenance reads or cleanup. Integrators (wallets,
indexers, treasury tooling) depend on coherent state transitions, emitted signals, and error behavior when inputs or
timing are invalid.

### Work to complete

1. Characterize the **intended protocol semantics** for *Regression tests: double-init and missing-config reads* in both success and failure cases, treating the **Summary** above as the authoritative scope statement.
2. Map which roles may participate, what each role must prove, and which operations must be impossible for non-authorized actors.
3. Enumerate edge cases driven by time (start, cliff, end, cancellation freeze), numeric ranges, and stream status combinations; capture expectations with tests or explicit audit exceptions.
4. Ensure externally visible behavior—persistent fields, contract errors, and emitted payloads—does not contradict documentation that integrators rely on.

### Definition of done

- An independent reader can confirm *Regression tests: double-init and missing-config reads* matches the stated intent using tests, traces, snapshots (where used), or written review notes.
- Documentation for this slice of the protocol is consistent with observable on-chain behavior after the change.
- The pull request explains verification performed, scope boundaries, and any residual assumptions or risks.

### Constraints for contributors

- Describe **outcomes**, **invariants**, and **evidence**, not a single “right” internal design unless the issue title already names a concrete subsystem.
- Prefer **observable** guarantees: state transitions, balances, authorization failures, emitted events, error classifications, and documentation that integrators rely on.
- If something cannot be tested automatically, capture the gap as **audit notes** with explicit rationale and residual risk.

## Requirements and context

- Must be **secure**, **well-tested**, and **documented**.
- Should be **gas-conscious** (Soroban budget) and **easy to audit**.

## Suggested execution

1. Fork the repo and create a branch:
   ```bash
   git checkout -b feature/tests-init-regression
   ```
2. Implement changes
   - **Contract / modules:** `contracts/stream/src/lib.rs`
   - **Comprehensive tests:** `contracts/stream/src/test.rs` and `contracts/stream/tests/integration_suite.rs`
   - **Documentation:** `docs/` (e.g. `streaming.md`, `security.md`, `events.md`)
   - Include **Rust doc comments** on public items (NatSpec-style clarity for auditors)
   - **Validate security assumptions** (CEI ordering, token trust model, auth paths)
3. **Test and commit**
   - Run `cargo test` in `contracts/stream` (and workspace if applicable)
   - Cover **edge cases** (overflow, TTL, paused/cancelled/completed)
   - Attach **test output** summary + **security notes** in the PR

### Example commit message

```
test(stream): cover double init and config absence
```

## Guidelines

- Aim for **≥95%** coverage on touched Rust modules (`cargo llvm-cov` or project-standard tooling).
- **Clear documentation** (protocol semantics + operator runbooks where relevant)
- **Timeframe:** 48 hours

++++++

---
type: Feature
title: Instance storage TTL bump policy (Config / NextStreamId)
labels: contracts, soroban, storage, reliability
assignees: ''
---

## Description

### Summary

Within the Fluxora streaming contract, this work tightens externally visible assurances for one focused concern. Treasury operators, recipient-facing applications, and third-party auditors must be able to reason about this area using only on-chain observables and published protocol documentation—without inferring hidden rules from how the implementation happens to be structured. The scenarios captured by this issue title must therefore have crisp success semantics, crisp failure semantics, and no silent drift between stored state, user-visible errors, and emitted events. Permissionless maintenance or read paths, if involved, must have clearly documented trust impact for operators. In practical terms, treat “Instance storage TTL bump policy (Config / NextStreamId)” as the scope boundary: everything materially related to that caption should be covered by evidence (automated or documented) in this change, and anything intentionally excluded should be called out with rationale and residual risk.

**Issue caption:** Instance storage TTL bump policy (Config / NextStreamId)

### Domain context

This repository hosts the Fluxora streaming contract on Soroban (Stellar). Streams lock funded assets in contract
storage, release entitlement over time according to schedule parameters (including cliffs), and expose explicit roles:
senders who fund and may adjust certain parameters, recipients who claim vested value, administrators who may intervene
under policy, and—where applicable—permissionless callers for maintenance reads or cleanup. Integrators (wallets,
indexers, treasury tooling) depend on coherent state transitions, emitted signals, and error behavior when inputs or
timing are invalid.

### Work to complete

1. Characterize the **intended protocol semantics** for *Instance storage TTL bump policy (Config / NextStreamId)* in both success and failure cases, treating the **Summary** above as the authoritative scope statement.
2. Map which roles may participate, what each role must prove, and which operations must be impossible for non-authorized actors.
3. Enumerate edge cases driven by time (start, cliff, end, cancellation freeze), numeric ranges, and stream status combinations; capture expectations with tests or explicit audit exceptions.
4. Ensure externally visible behavior—persistent fields, contract errors, and emitted payloads—does not contradict documentation that integrators rely on.

### Definition of done

- An independent reader can confirm *Instance storage TTL bump policy (Config / NextStreamId)* matches the stated intent using tests, traces, snapshots (where used), or written review notes.
- Documentation for this slice of the protocol is consistent with observable on-chain behavior after the change.
- The pull request explains verification performed, scope boundaries, and any residual assumptions or risks.

### Constraints for contributors

- Describe **outcomes**, **invariants**, and **evidence**, not a single “right” internal design unless the issue title already names a concrete subsystem.
- Prefer **observable** guarantees: state transitions, balances, authorization failures, emitted events, error classifications, and documentation that integrators rely on.
- If something cannot be tested automatically, capture the gap as **audit notes** with explicit rationale and residual risk.

## Requirements and context

- Must be **secure**, **well-tested**, and **documented**.
- Should be **gas-conscious** (Soroban budget) and **easy to audit**.

## Suggested execution

1. Fork the repo and create a branch:
   ```bash
   git checkout -b feature/instance-ttl-policy
   ```
2. Implement changes
   - **Contract / modules:** `contracts/stream/src/lib.rs`
   - **Comprehensive tests:** `contracts/stream/src/test.rs` and `contracts/stream/tests/integration_suite.rs`
   - **Documentation:** `docs/` (e.g. `streaming.md`, `security.md`, `events.md`)
   - Include **Rust doc comments** on public items (NatSpec-style clarity for auditors)
   - **Validate security assumptions** (CEI ordering, token trust model, auth paths)
3. **Test and commit**
   - Run `cargo test` in `contracts/stream` (and workspace if applicable)
   - Cover **edge cases** (overflow, TTL, paused/cancelled/completed)
   - Attach **test output** summary + **security notes** in the PR

### Example commit message

```
docs(stream): clarify instance TTL bump semantics
```

## Guidelines

- Aim for **≥95%** coverage on touched Rust modules (`cargo llvm-cov` or project-standard tooling).
- **Clear documentation** (protocol semantics + operator runbooks where relevant)
- **Timeframe:** 96 hours

++++++

---
type: Feature
title: Persistent stream TTL: extend-on-read/write invariants
labels: contracts, soroban, storage
assignees: ''
---

## Description

### Summary

Within the Fluxora streaming contract, this work tightens externally visible assurances for one focused concern. Treasury operators, recipient-facing applications, and third-party auditors must be able to reason about this area using only on-chain observables and published protocol documentation—without inferring hidden rules from how the implementation happens to be structured. The scenarios captured by this issue title must therefore have crisp success semantics, crisp failure semantics, and no silent drift between stored state, user-visible errors, and emitted events. Authorization boundaries for this slice must be explicit: who may trigger each path, and what proof they must supply. In practical terms, treat “Persistent stream TTL: extend-on-read/write invariants” as the scope boundary: everything materially related to that caption should be covered by evidence (automated or documented) in this change, and anything intentionally excluded should be called out with rationale and residual risk.

**Issue caption:** Persistent stream TTL: extend-on-read/write invariants

### Domain context

This repository hosts the Fluxora streaming contract on Soroban (Stellar). Streams lock funded assets in contract
storage, release entitlement over time according to schedule parameters (including cliffs), and expose explicit roles:
senders who fund and may adjust certain parameters, recipients who claim vested value, administrators who may intervene
under policy, and—where applicable—permissionless callers for maintenance reads or cleanup. Integrators (wallets,
indexers, treasury tooling) depend on coherent state transitions, emitted signals, and error behavior when inputs or
timing are invalid.

### Work to complete

1. Characterize the **intended protocol semantics** for *Persistent stream TTL: extend-on-read/write invariants* in both success and failure cases, treating the **Summary** above as the authoritative scope statement.
2. Map which roles may participate, what each role must prove, and which operations must be impossible for non-authorized actors.
3. Enumerate edge cases driven by time (start, cliff, end, cancellation freeze), numeric ranges, and stream status combinations; capture expectations with tests or explicit audit exceptions.
4. Ensure externally visible behavior—persistent fields, contract errors, and emitted payloads—does not contradict documentation that integrators rely on.

### Definition of done

- An independent reader can confirm *Persistent stream TTL: extend-on-read/write invariants* matches the stated intent using tests, traces, snapshots (where used), or written review notes.
- Documentation for this slice of the protocol is consistent with observable on-chain behavior after the change.
- The pull request explains verification performed, scope boundaries, and any residual assumptions or risks.

### Constraints for contributors

- Describe **outcomes**, **invariants**, and **evidence**, not a single “right” internal design unless the issue title already names a concrete subsystem.
- Prefer **observable** guarantees: state transitions, balances, authorization failures, emitted events, error classifications, and documentation that integrators rely on.
- If something cannot be tested automatically, capture the gap as **audit notes** with explicit rationale and residual risk.

## Requirements and context

- Must be **secure**, **well-tested**, and **documented**.
- Should be **gas-conscious** (Soroban budget) and **easy to audit**.

## Suggested execution

1. Fork the repo and create a branch:
   ```bash
   git checkout -b feature/persistent-ttl-invariants
   ```
2. Implement changes
   - **Contract / modules:** `contracts/stream/src/lib.rs`
   - **Comprehensive tests:** `contracts/stream/src/test.rs` and `contracts/stream/tests/integration_suite.rs`
   - **Documentation:** `docs/` (e.g. `streaming.md`, `security.md`, `events.md`)
   - Include **Rust doc comments** on public items (NatSpec-style clarity for auditors)
   - **Validate security assumptions** (CEI ordering, token trust model, auth paths)
3. **Test and commit**
   - Run `cargo test` in `contracts/stream` (and workspace if applicable)
   - Cover **edge cases** (overflow, TTL, paused/cancelled/completed)
   - Attach **test output** summary + **security notes** in the PR

### Example commit message

```
test(stream): cover persistent stream TTL extensions
```

## Guidelines

- Aim for **≥95%** coverage on touched Rust modules (`cargo llvm-cov` or project-standard tooling).
- **Clear documentation** (protocol semantics + operator runbooks where relevant)
- **Timeframe:** 96 hours

++++++

---
type: Task
title: Recipient index: sorted insertion by stream_id
labels: contracts, soroban, indexing
assignees: ''
---

## Description

### Summary

Within the Fluxora streaming contract, this work tightens externally visible assurances for one focused concern. Treasury operators, recipient-facing applications, and third-party auditors must be able to reason about this area using only on-chain observables and published protocol documentation—without inferring hidden rules from how the implementation happens to be structured. The scenarios captured by this issue title must therefore have crisp success semantics, crisp failure semantics, and no silent drift between stored state, user-visible errors, and emitted events. Emitted signals and error classifications must remain coherent for indexers and wallets building on this protocol surface. In practical terms, treat “Recipient index: sorted insertion by stream_id” as the scope boundary: everything materially related to that caption should be covered by evidence (automated or documented) in this change, and anything intentionally excluded should be called out with rationale and residual risk.

**Issue caption:** Recipient index: sorted insertion by stream_id

### Domain context

This repository hosts the Fluxora streaming contract on Soroban (Stellar). Streams lock funded assets in contract
storage, release entitlement over time according to schedule parameters (including cliffs), and expose explicit roles:
senders who fund and may adjust certain parameters, recipients who claim vested value, administrators who may intervene
under policy, and—where applicable—permissionless callers for maintenance reads or cleanup. Integrators (wallets,
indexers, treasury tooling) depend on coherent state transitions, emitted signals, and error behavior when inputs or
timing are invalid.

### Work to complete

1. Characterize the **intended protocol semantics** for *Recipient index: sorted insertion by stream_id* in both success and failure cases, treating the **Summary** above as the authoritative scope statement.
2. Map which roles may participate, what each role must prove, and which operations must be impossible for non-authorized actors.
3. Enumerate edge cases driven by time (start, cliff, end, cancellation freeze), numeric ranges, and stream status combinations; capture expectations with tests or explicit audit exceptions.
4. Ensure externally visible behavior—persistent fields, contract errors, and emitted payloads—does not contradict documentation that integrators rely on.

### Definition of done

- An independent reader can confirm *Recipient index: sorted insertion by stream_id* matches the stated intent using tests, traces, snapshots (where used), or written review notes.
- Documentation for this slice of the protocol is consistent with observable on-chain behavior after the change.
- The pull request explains verification performed, scope boundaries, and any residual assumptions or risks.

### Constraints for contributors

- Describe **outcomes**, **invariants**, and **evidence**, not a single “right” internal design unless the issue title already names a concrete subsystem.
- Prefer **observable** guarantees: state transitions, balances, authorization failures, emitted events, error classifications, and documentation that integrators rely on.
- If something cannot be tested automatically, capture the gap as **audit notes** with explicit rationale and residual risk.

## Requirements and context

- Must be **secure**, **well-tested**, and **documented**.
- Should be **gas-conscious** (Soroban budget) and **easy to audit**.

## Suggested execution

1. Fork the repo and create a branch:
   ```bash
   git checkout -b feature/recipient-index-insert
   ```
2. Implement changes
   - **Contract / modules:** `contracts/stream/src/lib.rs`
   - **Comprehensive tests:** `contracts/stream/src/test.rs` and `contracts/stream/tests/integration_suite.rs`
   - **Documentation:** `docs/` (e.g. `streaming.md`, `security.md`, `events.md`)
   - Include **Rust doc comments** on public items (NatSpec-style clarity for auditors)
   - **Validate security assumptions** (CEI ordering, token trust model, auth paths)
3. **Test and commit**
   - Run `cargo test` in `contracts/stream` (and workspace if applicable)
   - Cover **edge cases** (overflow, TTL, paused/cancelled/completed)
   - Attach **test output** summary + **security notes** in the PR

### Example commit message

```
test(stream): cover recipient index insertion ordering
```

## Guidelines

- Aim for **≥95%** coverage on touched Rust modules (`cargo llvm-cov` or project-standard tooling).
- **Clear documentation** (protocol semantics + operator runbooks where relevant)
- **Timeframe:** 96 hours

++++++

---
type: Feature
title: Recipient index: removal on close_completed_stream
labels: contracts, soroban, indexing
assignees: ''
---

## Description

### Summary

Within the Fluxora streaming contract, this work tightens externally visible assurances for one focused concern. Treasury operators, recipient-facing applications, and third-party auditors must be able to reason about this area using only on-chain observables and published protocol documentation—without inferring hidden rules from how the implementation happens to be structured. The scenarios captured by this issue title must therefore have crisp success semantics, crisp failure semantics, and no silent drift between stored state, user-visible errors, and emitted events. Time boundaries (start, cliff, end, cancellation freeze) and numeric ranges must be enumerated so entitlement cannot be misread under edge timing. In practical terms, treat “Recipient index: removal on close_completed_stream” as the scope boundary: everything materially related to that caption should be covered by evidence (automated or documented) in this change, and anything intentionally excluded should be called out with rationale and residual risk.

**Issue caption:** Recipient index: removal on close_completed_stream

### Domain context

This repository hosts the Fluxora streaming contract on Soroban (Stellar). Streams lock funded assets in contract
storage, release entitlement over time according to schedule parameters (including cliffs), and expose explicit roles:
senders who fund and may adjust certain parameters, recipients who claim vested value, administrators who may intervene
under policy, and—where applicable—permissionless callers for maintenance reads or cleanup. Integrators (wallets,
indexers, treasury tooling) depend on coherent state transitions, emitted signals, and error behavior when inputs or
timing are invalid.

### Work to complete

1. Characterize the **intended protocol semantics** for *Recipient index: removal on close_completed_stream* in both success and failure cases, treating the **Summary** above as the authoritative scope statement.
2. Map which roles may participate, what each role must prove, and which operations must be impossible for non-authorized actors.
3. Enumerate edge cases driven by time (start, cliff, end, cancellation freeze), numeric ranges, and stream status combinations; capture expectations with tests or explicit audit exceptions.
4. Ensure externally visible behavior—persistent fields, contract errors, and emitted payloads—does not contradict documentation that integrators rely on.

### Definition of done

- An independent reader can confirm *Recipient index: removal on close_completed_stream* matches the stated intent using tests, traces, snapshots (where used), or written review notes.
- Documentation for this slice of the protocol is consistent with observable on-chain behavior after the change.
- The pull request explains verification performed, scope boundaries, and any residual assumptions or risks.

### Constraints for contributors

- Describe **outcomes**, **invariants**, and **evidence**, not a single “right” internal design unless the issue title already names a concrete subsystem.
- Prefer **observable** guarantees: state transitions, balances, authorization failures, emitted events, error classifications, and documentation that integrators rely on.
- If something cannot be tested automatically, capture the gap as **audit notes** with explicit rationale and residual risk.

## Requirements and context

- Must be **secure**, **well-tested**, and **documented**.
- Should be **gas-conscious** (Soroban budget) and **easy to audit**.

## Suggested execution

1. Fork the repo and create a branch:
   ```bash
   git checkout -b feature/recipient-index-close
   ```
2. Implement changes
   - **Contract / modules:** `contracts/stream/src/lib.rs`
   - **Comprehensive tests:** `contracts/stream/src/test.rs` and `contracts/stream/tests/integration_suite.rs`
   - **Documentation:** `docs/` (e.g. `streaming.md`, `security.md`, `events.md`)
   - Include **Rust doc comments** on public items (NatSpec-style clarity for auditors)
   - **Validate security assumptions** (CEI ordering, token trust model, auth paths)
3. **Test and commit**
   - Run `cargo test` in `contracts/stream` (and workspace if applicable)
   - Cover **edge cases** (overflow, TTL, paused/cancelled/completed)
   - Attach **test output** summary + **security notes** in the PR

### Example commit message

```
fix(stream): verify recipient index removal on close
```

## Guidelines

- Aim for **≥95%** coverage on touched Rust modules (`cargo llvm-cov` or project-standard tooling).
- **Clear documentation** (protocol semantics + operator runbooks where relevant)
- **Timeframe:** 96 hours

++++++

---
type: Feature
title: View helpers: get_recipient_streams / get_recipient_stream_count
labels: contracts, soroban, views
assignees: ''
---

## Description

### Summary

Within the Fluxora streaming contract, this work tightens externally visible assurances for one focused concern. Treasury operators, recipient-facing applications, and third-party auditors must be able to reason about this area using only on-chain observables and published protocol documentation—without inferring hidden rules from how the implementation happens to be structured. The scenarios captured by this issue title must therefore have crisp success semantics, crisp failure semantics, and no silent drift between stored state, user-visible errors, and emitted events. Time boundaries (start, cliff, end, cancellation freeze) and numeric ranges must be enumerated so entitlement cannot be misread under edge timing. In practical terms, treat “View helpers: get_recipient_streams / get_recipient_stream_count” as the scope boundary: everything materially related to that caption should be covered by evidence (automated or documented) in this change, and anything intentionally excluded should be called out with rationale and residual risk.

**Issue caption:** View helpers: get_recipient_streams / get_recipient_stream_count

### Domain context

This repository hosts the Fluxora streaming contract on Soroban (Stellar). Streams lock funded assets in contract
storage, release entitlement over time according to schedule parameters (including cliffs), and expose explicit roles:
senders who fund and may adjust certain parameters, recipients who claim vested value, administrators who may intervene
under policy, and—where applicable—permissionless callers for maintenance reads or cleanup. Integrators (wallets,
indexers, treasury tooling) depend on coherent state transitions, emitted signals, and error behavior when inputs or
timing are invalid.

### Work to complete

1. Characterize the **intended protocol semantics** for *View helpers: get_recipient_streams / get_recipient_stream_count* in both success and failure cases, treating the **Summary** above as the authoritative scope statement.
2. Map which roles may participate, what each role must prove, and which operations must be impossible for non-authorized actors.
3. Enumerate edge cases driven by time (start, cliff, end, cancellation freeze), numeric ranges, and stream status combinations; capture expectations with tests or explicit audit exceptions.
4. Ensure externally visible behavior—persistent fields, contract errors, and emitted payloads—does not contradict documentation that integrators rely on.

### Definition of done

- An independent reader can confirm *View helpers: get_recipient_streams / get_recipient_stream_count* matches the stated intent using tests, traces, snapshots (where used), or written review notes.
- Documentation for this slice of the protocol is consistent with observable on-chain behavior after the change.
- The pull request explains verification performed, scope boundaries, and any residual assumptions or risks.

### Constraints for contributors

- Describe **outcomes**, **invariants**, and **evidence**, not a single “right” internal design unless the issue title already names a concrete subsystem.
- Prefer **observable** guarantees: state transitions, balances, authorization failures, emitted events, error classifications, and documentation that integrators rely on.
- If something cannot be tested automatically, capture the gap as **audit notes** with explicit rationale and residual risk.

## Requirements and context

- Must be **secure**, **well-tested**, and **documented**.
- Should be **gas-conscious** (Soroban budget) and **easy to audit**.

## Suggested execution

1. Fork the repo and create a branch:
   ```bash
   git checkout -b feature/recipient-views
   ```
2. Implement changes
   - **Contract / modules:** `contracts/stream/src/lib.rs`
   - **Comprehensive tests:** `contracts/stream/src/test.rs` and `contracts/stream/tests/integration_suite.rs`
   - **Documentation:** `docs/` (e.g. `streaming.md`, `security.md`, `events.md`)
   - Include **Rust doc comments** on public items (NatSpec-style clarity for auditors)
   - **Validate security assumptions** (CEI ordering, token trust model, auth paths)
3. **Test and commit**
   - Run `cargo test` in `contracts/stream` (and workspace if applicable)
   - Cover **edge cases** (overflow, TTL, paused/cancelled/completed)
   - Attach **test output** summary + **security notes** in the PR

### Example commit message

```
test(stream): align recipient enumeration views
```

## Guidelines

- Aim for **≥95%** coverage on touched Rust modules (`cargo llvm-cov` or project-standard tooling).
- **Clear documentation** (protocol semantics + operator runbooks where relevant)
- **Timeframe:** 96 hours

++++++

---
type: Feature
title: create_stream: deposit, rate, and schedule validation matrix
labels: contracts, soroban, validation
assignees: ''
---

## Description

### Summary

Within the Fluxora streaming contract, this work tightens externally visible assurances for one focused concern. Treasury operators, recipient-facing applications, and third-party auditors must be able to reason about this area using only on-chain observables and published protocol documentation—without inferring hidden rules from how the implementation happens to be structured. The scenarios captured by this issue title must therefore have crisp success semantics, crisp failure semantics, and no silent drift between stored state, user-visible errors, and emitted events. Permissionless maintenance or read paths, if involved, must have clearly documented trust impact for operators. In practical terms, treat “create_stream: deposit, rate, and schedule validation matrix” as the scope boundary: everything materially related to that caption should be covered by evidence (automated or documented) in this change, and anything intentionally excluded should be called out with rationale and residual risk.

**Issue caption:** create_stream: deposit, rate, and schedule validation matrix

### Domain context

This repository hosts the Fluxora streaming contract on Soroban (Stellar). Streams lock funded assets in contract
storage, release entitlement over time according to schedule parameters (including cliffs), and expose explicit roles:
senders who fund and may adjust certain parameters, recipients who claim vested value, administrators who may intervene
under policy, and—where applicable—permissionless callers for maintenance reads or cleanup. Integrators (wallets,
indexers, treasury tooling) depend on coherent state transitions, emitted signals, and error behavior when inputs or
timing are invalid.

### Work to complete

1. Characterize the **intended protocol semantics** for *create_stream: deposit, rate, and schedule validation matrix* in both success and failure cases, treating the **Summary** above as the authoritative scope statement.
2. Map which roles may participate, what each role must prove, and which operations must be impossible for non-authorized actors.
3. Enumerate edge cases driven by time (start, cliff, end, cancellation freeze), numeric ranges, and stream status combinations; capture expectations with tests or explicit audit exceptions.
4. Ensure externally visible behavior—persistent fields, contract errors, and emitted payloads—does not contradict documentation that integrators rely on.

### Definition of done

- An independent reader can confirm *create_stream: deposit, rate, and schedule validation matrix* matches the stated intent using tests, traces, snapshots (where used), or written review notes.
- Documentation for this slice of the protocol is consistent with observable on-chain behavior after the change.
- The pull request explains verification performed, scope boundaries, and any residual assumptions or risks.

### Constraints for contributors

- Describe **outcomes**, **invariants**, and **evidence**, not a single “right” internal design unless the issue title already names a concrete subsystem.
- Prefer **observable** guarantees: state transitions, balances, authorization failures, emitted events, error classifications, and documentation that integrators rely on.
- If something cannot be tested automatically, capture the gap as **audit notes** with explicit rationale and residual risk.

## Requirements and context

- Must be **secure**, **well-tested**, and **documented**.
- Should be **gas-conscious** (Soroban budget) and **easy to audit**.

## Suggested execution

1. Fork the repo and create a branch:
   ```bash
   git checkout -b feature/create-stream-validation-matrix
   ```
2. Implement changes
   - **Contract / modules:** `contracts/stream/src/lib.rs`
   - **Comprehensive tests:** `contracts/stream/src/test.rs` and `contracts/stream/tests/integration_suite.rs`
   - **Documentation:** `docs/` (e.g. `streaming.md`, `security.md`, `events.md`)
   - Include **Rust doc comments** on public items (NatSpec-style clarity for auditors)
   - **Validate security assumptions** (CEI ordering, token trust model, auth paths)
3. **Test and commit**
   - Run `cargo test` in `contracts/stream` (and workspace if applicable)
   - Cover **edge cases** (overflow, TTL, paused/cancelled/completed)
   - Attach **test output** summary + **security notes** in the PR

### Example commit message

```
test(stream): expand create_stream validation coverage
```

## Guidelines

- Aim for **≥95%** coverage on touched Rust modules (`cargo llvm-cov` or project-standard tooling).
- **Clear documentation** (protocol semantics + operator runbooks where relevant)
- **Timeframe:** 96 hours

++++++

---
type: Feature
title: create_stream: disallow sender == recipient
labels: contracts, soroban, validation
assignees: ''
---

## Description

### Summary

Within the Fluxora streaming contract, this work tightens externally visible assurances for one focused concern. Treasury operators, recipient-facing applications, and third-party auditors must be able to reason about this area using only on-chain observables and published protocol documentation—without inferring hidden rules from how the implementation happens to be structured. The scenarios captured by this issue title must therefore have crisp success semantics, crisp failure semantics, and no silent drift between stored state, user-visible errors, and emitted events. Authorization boundaries for this slice must be explicit: who may trigger each path, and what proof they must supply. In practical terms, treat “create_stream: disallow sender == recipient” as the scope boundary: everything materially related to that caption should be covered by evidence (automated or documented) in this change, and anything intentionally excluded should be called out with rationale and residual risk.

**Issue caption:** create_stream: disallow sender == recipient

### Domain context

This repository hosts the Fluxora streaming contract on Soroban (Stellar). Streams lock funded assets in contract
storage, release entitlement over time according to schedule parameters (including cliffs), and expose explicit roles:
senders who fund and may adjust certain parameters, recipients who claim vested value, administrators who may intervene
under policy, and—where applicable—permissionless callers for maintenance reads or cleanup. Integrators (wallets,
indexers, treasury tooling) depend on coherent state transitions, emitted signals, and error behavior when inputs or
timing are invalid.

### Work to complete

1. Characterize the **intended protocol semantics** for *create_stream: disallow sender == recipient* in both success and failure cases, treating the **Summary** above as the authoritative scope statement.
2. Map which roles may participate, what each role must prove, and which operations must be impossible for non-authorized actors.
3. Enumerate edge cases driven by time (start, cliff, end, cancellation freeze), numeric ranges, and stream status combinations; capture expectations with tests or explicit audit exceptions.
4. Ensure externally visible behavior—persistent fields, contract errors, and emitted payloads—does not contradict documentation that integrators rely on.

### Definition of done

- An independent reader can confirm *create_stream: disallow sender == recipient* matches the stated intent using tests, traces, snapshots (where used), or written review notes.
- Documentation for this slice of the protocol is consistent with observable on-chain behavior after the change.
- The pull request explains verification performed, scope boundaries, and any residual assumptions or risks.

### Constraints for contributors

- Describe **outcomes**, **invariants**, and **evidence**, not a single “right” internal design unless the issue title already names a concrete subsystem.
- Prefer **observable** guarantees: state transitions, balances, authorization failures, emitted events, error classifications, and documentation that integrators rely on.
- If something cannot be tested automatically, capture the gap as **audit notes** with explicit rationale and residual risk.

## Requirements and context

- Must be **secure**, **well-tested**, and **documented**.
- Should be **gas-conscious** (Soroban budget) and **easy to audit**.

## Suggested execution

1. Fork the repo and create a branch:
   ```bash
   git checkout -b feature/create-stream-self-stream
   ```
2. Implement changes
   - **Contract / modules:** `contracts/stream/src/lib.rs`
   - **Comprehensive tests:** `contracts/stream/src/test.rs` and `contracts/stream/tests/integration_suite.rs`
   - **Documentation:** `docs/` (e.g. `streaming.md`, `security.md`, `events.md`)
   - Include **Rust doc comments** on public items (NatSpec-style clarity for auditors)
   - **Validate security assumptions** (CEI ordering, token trust model, auth paths)
3. **Test and commit**
   - Run `cargo test` in `contracts/stream` (and workspace if applicable)
   - Cover **edge cases** (overflow, TTL, paused/cancelled/completed)
   - Attach **test output** summary + **security notes** in the PR

### Example commit message

```
test(stream): reject sender equals recipient
```

## Guidelines

- Aim for **≥95%** coverage on touched Rust modules (`cargo llvm-cov` or project-standard tooling).
- **Clear documentation** (protocol semantics + operator runbooks where relevant)
- **Timeframe:** 96 hours

++++++

---
type: Task
title: create_stream: past start_time rejected
labels: contracts, soroban, validation
assignees: ''
---

## Description

### Summary

Within the Fluxora streaming contract, this work tightens externally visible assurances for one focused concern. Treasury operators, recipient-facing applications, and third-party auditors must be able to reason about this area using only on-chain observables and published protocol documentation—without inferring hidden rules from how the implementation happens to be structured. The scenarios captured by this issue title must therefore have crisp success semantics, crisp failure semantics, and no silent drift between stored state, user-visible errors, and emitted events. Time boundaries (start, cliff, end, cancellation freeze) and numeric ranges must be enumerated so entitlement cannot be misread under edge timing. In practical terms, treat “create_stream: past start_time rejected” as the scope boundary: everything materially related to that caption should be covered by evidence (automated or documented) in this change, and anything intentionally excluded should be called out with rationale and residual risk.

**Issue caption:** create_stream: past start_time rejected

### Domain context

This repository hosts the Fluxora streaming contract on Soroban (Stellar). Streams lock funded assets in contract
storage, release entitlement over time according to schedule parameters (including cliffs), and expose explicit roles:
senders who fund and may adjust certain parameters, recipients who claim vested value, administrators who may intervene
under policy, and—where applicable—permissionless callers for maintenance reads or cleanup. Integrators (wallets,
indexers, treasury tooling) depend on coherent state transitions, emitted signals, and error behavior when inputs or
timing are invalid.

### Work to complete

1. Characterize the **intended protocol semantics** for *create_stream: past start_time rejected* in both success and failure cases, treating the **Summary** above as the authoritative scope statement.
2. Map which roles may participate, what each role must prove, and which operations must be impossible for non-authorized actors.
3. Enumerate edge cases driven by time (start, cliff, end, cancellation freeze), numeric ranges, and stream status combinations; capture expectations with tests or explicit audit exceptions.
4. Ensure externally visible behavior—persistent fields, contract errors, and emitted payloads—does not contradict documentation that integrators rely on.

### Definition of done

- An independent reader can confirm *create_stream: past start_time rejected* matches the stated intent using tests, traces, snapshots (where used), or written review notes.
- Documentation for this slice of the protocol is consistent with observable on-chain behavior after the change.
- The pull request explains verification performed, scope boundaries, and any residual assumptions or risks.

### Constraints for contributors

- Describe **outcomes**, **invariants**, and **evidence**, not a single “right” internal design unless the issue title already names a concrete subsystem.
- Prefer **observable** guarantees: state transitions, balances, authorization failures, emitted events, error classifications, and documentation that integrators rely on.
- If something cannot be tested automatically, capture the gap as **audit notes** with explicit rationale and residual risk.

## Requirements and context

- Must be **secure**, **well-tested**, and **documented**.
- Should be **gas-conscious** (Soroban budget) and **easy to audit**.

## Suggested execution

1. Fork the repo and create a branch:
   ```bash
   git checkout -b feature/create-stream-past-start
   ```
2. Implement changes
   - **Contract / modules:** `contracts/stream/src/lib.rs`
   - **Comprehensive tests:** `contracts/stream/src/test.rs` and `contracts/stream/tests/integration_suite.rs`
   - **Documentation:** `docs/` (e.g. `streaming.md`, `security.md`, `events.md`)
   - Include **Rust doc comments** on public items (NatSpec-style clarity for auditors)
   - **Validate security assumptions** (CEI ordering, token trust model, auth paths)
3. **Test and commit**
   - Run `cargo test` in `contracts/stream` (and workspace if applicable)
   - Cover **edge cases** (overflow, TTL, paused/cancelled/completed)
   - Attach **test output** summary + **security notes** in the PR

### Example commit message

```
test(stream): reject past start_time
```

## Guidelines

- Aim for **≥95%** coverage on touched Rust modules (`cargo llvm-cov` or project-standard tooling).
- **Clear documentation** (protocol semantics + operator runbooks where relevant)
- **Timeframe:** 96 hours

++++++

---
type: Feature
title: create_streams: batch atomicity and single auth
labels: contracts, soroban, batch, gas
assignees: ''
---

## Description

### Summary

Within the Fluxora streaming contract, this work tightens externally visible assurances for one focused concern. Treasury operators, recipient-facing applications, and third-party auditors must be able to reason about this area using only on-chain observables and published protocol documentation—without inferring hidden rules from how the implementation happens to be structured. The scenarios captured by this issue title must therefore have crisp success semantics, crisp failure semantics, and no silent drift between stored state, user-visible errors, and emitted events. Economic conservation and payout ordering assumptions must be defensible to a treasury or auditor without reference to internal layout. In practical terms, treat “create_streams: batch atomicity and single auth” as the scope boundary: everything materially related to that caption should be covered by evidence (automated or documented) in this change, and anything intentionally excluded should be called out with rationale and residual risk.

**Issue caption:** create_streams: batch atomicity and single auth

### Domain context

This repository hosts the Fluxora streaming contract on Soroban (Stellar). Streams lock funded assets in contract
storage, release entitlement over time according to schedule parameters (including cliffs), and expose explicit roles:
senders who fund and may adjust certain parameters, recipients who claim vested value, administrators who may intervene
under policy, and—where applicable—permissionless callers for maintenance reads or cleanup. Integrators (wallets,
indexers, treasury tooling) depend on coherent state transitions, emitted signals, and error behavior when inputs or
timing are invalid.

### Work to complete

1. Characterize the **intended protocol semantics** for *create_streams: batch atomicity and single auth* in both success and failure cases, treating the **Summary** above as the authoritative scope statement.
2. Map which roles may participate, what each role must prove, and which operations must be impossible for non-authorized actors.
3. Enumerate edge cases driven by time (start, cliff, end, cancellation freeze), numeric ranges, and stream status combinations; capture expectations with tests or explicit audit exceptions.
4. Ensure externally visible behavior—persistent fields, contract errors, and emitted payloads—does not contradict documentation that integrators rely on.

### Definition of done

- An independent reader can confirm *create_streams: batch atomicity and single auth* matches the stated intent using tests, traces, snapshots (where used), or written review notes.
- Documentation for this slice of the protocol is consistent with observable on-chain behavior after the change.
- The pull request explains verification performed, scope boundaries, and any residual assumptions or risks.

### Constraints for contributors

- Describe **outcomes**, **invariants**, and **evidence**, not a single “right” internal design unless the issue title already names a concrete subsystem.
- Prefer **observable** guarantees: state transitions, balances, authorization failures, emitted events, error classifications, and documentation that integrators rely on.
- If something cannot be tested automatically, capture the gap as **audit notes** with explicit rationale and residual risk.

## Requirements and context

- Must be **secure**, **well-tested**, and **documented**.
- Should be **gas-conscious** (Soroban budget) and **easy to audit**.

## Suggested execution

1. Fork the repo and create a branch:
   ```bash
   git checkout -b feature/create-streams-batch
   ```
2. Implement changes
   - **Contract / modules:** `contracts/stream/src/lib.rs`
   - **Comprehensive tests:** `contracts/stream/src/test.rs` and `contracts/stream/tests/integration_suite.rs`
   - **Documentation:** `docs/` (e.g. `streaming.md`, `security.md`, `events.md`)
   - Include **Rust doc comments** on public items (NatSpec-style clarity for auditors)
   - **Validate security assumptions** (CEI ordering, token trust model, auth paths)
3. **Test and commit**
   - Run `cargo test` in `contracts/stream` (and workspace if applicable)
   - Cover **edge cases** (overflow, TTL, paused/cancelled/completed)
   - Attach **test output** summary + **security notes** in the PR

### Example commit message

```
test(stream): harden create_streams atomicity
```

## Guidelines

- Aim for **≥95%** coverage on touched Rust modules (`cargo llvm-cov` or project-standard tooling).
- **Clear documentation** (protocol semantics + operator runbooks where relevant)
- **Timeframe:** 96 hours

++++++

---
type: Feature
title: create_streams: total deposit overflow protection
labels: contracts, soroban, safety
assignees: ''
---

## Description

### Summary

Within the Fluxora streaming contract, this work tightens externally visible assurances for one focused concern. Treasury operators, recipient-facing applications, and third-party auditors must be able to reason about this area using only on-chain observables and published protocol documentation—without inferring hidden rules from how the implementation happens to be structured. The scenarios captured by this issue title must therefore have crisp success semantics, crisp failure semantics, and no silent drift between stored state, user-visible errors, and emitted events. Emitted signals and error classifications must remain coherent for indexers and wallets building on this protocol surface. In practical terms, treat “create_streams: total deposit overflow protection” as the scope boundary: everything materially related to that caption should be covered by evidence (automated or documented) in this change, and anything intentionally excluded should be called out with rationale and residual risk.

**Issue caption:** create_streams: total deposit overflow protection

### Domain context

This repository hosts the Fluxora streaming contract on Soroban (Stellar). Streams lock funded assets in contract
storage, release entitlement over time according to schedule parameters (including cliffs), and expose explicit roles:
senders who fund and may adjust certain parameters, recipients who claim vested value, administrators who may intervene
under policy, and—where applicable—permissionless callers for maintenance reads or cleanup. Integrators (wallets,
indexers, treasury tooling) depend on coherent state transitions, emitted signals, and error behavior when inputs or
timing are invalid.

### Work to complete

1. Characterize the **intended protocol semantics** for *create_streams: total deposit overflow protection* in both success and failure cases, treating the **Summary** above as the authoritative scope statement.
2. Map which roles may participate, what each role must prove, and which operations must be impossible for non-authorized actors.
3. Enumerate edge cases driven by time (start, cliff, end, cancellation freeze), numeric ranges, and stream status combinations; capture expectations with tests or explicit audit exceptions.
4. Ensure externally visible behavior—persistent fields, contract errors, and emitted payloads—does not contradict documentation that integrators rely on.

### Definition of done

- An independent reader can confirm *create_streams: total deposit overflow protection* matches the stated intent using tests, traces, snapshots (where used), or written review notes.
- Documentation for this slice of the protocol is consistent with observable on-chain behavior after the change.
- The pull request explains verification performed, scope boundaries, and any residual assumptions or risks.

### Constraints for contributors

- Describe **outcomes**, **invariants**, and **evidence**, not a single “right” internal design unless the issue title already names a concrete subsystem.
- Prefer **observable** guarantees: state transitions, balances, authorization failures, emitted events, error classifications, and documentation that integrators rely on.
- If something cannot be tested automatically, capture the gap as **audit notes** with explicit rationale and residual risk.

## Requirements and context

- Must be **secure**, **well-tested**, and **documented**.
- Should be **gas-conscious** (Soroban budget) and **easy to audit**.

## Suggested execution

1. Fork the repo and create a branch:
   ```bash
   git checkout -b feature/create-streams-overflow
   ```
2. Implement changes
   - **Contract / modules:** `contracts/stream/src/lib.rs`
   - **Comprehensive tests:** `contracts/stream/src/test.rs` and `contracts/stream/tests/integration_suite.rs`
   - **Documentation:** `docs/` (e.g. `streaming.md`, `security.md`, `events.md`)
   - Include **Rust doc comments** on public items (NatSpec-style clarity for auditors)
   - **Validate security assumptions** (CEI ordering, token trust model, auth paths)
3. **Test and commit**
   - Run `cargo test` in `contracts/stream` (and workspace if applicable)
   - Cover **edge cases** (overflow, TTL, paused/cancelled/completed)
   - Attach **test output** summary + **security notes** in the PR

### Example commit message

```
test(stream): cover batch deposit overflow
```

## Guidelines

- Aim for **≥95%** coverage on touched Rust modules (`cargo llvm-cov` or project-standard tooling).
- **Clear documentation** (protocol semantics + operator runbooks where relevant)
- **Timeframe:** 96 hours

++++++

---
type: Feature
title: Global pause: ContractPaused blocks new stream creation
labels: contracts, soroban, governance
assignees: ''
---

## Description

### Summary

Within the Fluxora streaming contract, this work tightens externally visible assurances for one focused concern. Treasury operators, recipient-facing applications, and third-party auditors must be able to reason about this area using only on-chain observables and published protocol documentation—without inferring hidden rules from how the implementation happens to be structured. The scenarios captured by this issue title must therefore have crisp success semantics, crisp failure semantics, and no silent drift between stored state, user-visible errors, and emitted events. Time boundaries (start, cliff, end, cancellation freeze) and numeric ranges must be enumerated so entitlement cannot be misread under edge timing. In practical terms, treat “Global pause: ContractPaused blocks new stream creation” as the scope boundary: everything materially related to that caption should be covered by evidence (automated or documented) in this change, and anything intentionally excluded should be called out with rationale and residual risk.

**Issue caption:** Global pause: ContractPaused blocks new stream creation

### Domain context

This repository hosts the Fluxora streaming contract on Soroban (Stellar). Streams lock funded assets in contract
storage, release entitlement over time according to schedule parameters (including cliffs), and expose explicit roles:
senders who fund and may adjust certain parameters, recipients who claim vested value, administrators who may intervene
under policy, and—where applicable—permissionless callers for maintenance reads or cleanup. Integrators (wallets,
indexers, treasury tooling) depend on coherent state transitions, emitted signals, and error behavior when inputs or
timing are invalid.

### Work to complete

1. Characterize the **intended protocol semantics** for *Global pause: ContractPaused blocks new stream creation* in both success and failure cases, treating the **Summary** above as the authoritative scope statement.
2. Map which roles may participate, what each role must prove, and which operations must be impossible for non-authorized actors.
3. Enumerate edge cases driven by time (start, cliff, end, cancellation freeze), numeric ranges, and stream status combinations; capture expectations with tests or explicit audit exceptions.
4. Ensure externally visible behavior—persistent fields, contract errors, and emitted payloads—does not contradict documentation that integrators rely on.

### Definition of done

- An independent reader can confirm *Global pause: ContractPaused blocks new stream creation* matches the stated intent using tests, traces, snapshots (where used), or written review notes.
- Documentation for this slice of the protocol is consistent with observable on-chain behavior after the change.
- The pull request explains verification performed, scope boundaries, and any residual assumptions or risks.

### Constraints for contributors

- Describe **outcomes**, **invariants**, and **evidence**, not a single “right” internal design unless the issue title already names a concrete subsystem.
- Prefer **observable** guarantees: state transitions, balances, authorization failures, emitted events, error classifications, and documentation that integrators rely on.
- If something cannot be tested automatically, capture the gap as **audit notes** with explicit rationale and residual risk.

## Requirements and context

- Must be **secure**, **well-tested**, and **documented**.
- Should be **gas-conscious** (Soroban budget) and **easy to audit**.

## Suggested execution

1. Fork the repo and create a branch:
   ```bash
   git checkout -b feature/global-pause-create
   ```
2. Implement changes
   - **Contract / modules:** `contracts/stream/src/lib.rs`
   - **Comprehensive tests:** `contracts/stream/src/test.rs` and `contracts/stream/tests/integration_suite.rs`
   - **Documentation:** `docs/` (e.g. `streaming.md`, `security.md`, `events.md`)
   - Include **Rust doc comments** on public items (NatSpec-style clarity for auditors)
   - **Validate security assumptions** (CEI ordering, token trust model, auth paths)
3. **Test and commit**
   - Run `cargo test` in `contracts/stream` (and workspace if applicable)
   - Cover **edge cases** (overflow, TTL, paused/cancelled/completed)
   - Attach **test output** summary + **security notes** in the PR

### Example commit message

```
test(stream): global pause blocks creation
```

## Guidelines

- Aim for **≥95%** coverage on touched Rust modules (`cargo llvm-cov` or project-standard tooling).
- **Clear documentation** (protocol semantics + operator runbooks where relevant)
- **Timeframe:** 96 hours

++++++

---
type: Feature
title: pause_stream / resume_stream: sender authorization paths
labels: contracts, soroban, lifecycle
assignees: ''
---

## Description

### Summary

Within the Fluxora streaming contract, this work tightens externally visible assurances for one focused concern. Treasury operators, recipient-facing applications, and third-party auditors must be able to reason about this area using only on-chain observables and published protocol documentation—without inferring hidden rules from how the implementation happens to be structured. The scenarios captured by this issue title must therefore have crisp success semantics, crisp failure semantics, and no silent drift between stored state, user-visible errors, and emitted events. Authorization boundaries for this slice must be explicit: who may trigger each path, and what proof they must supply. In practical terms, treat “pause_stream / resume_stream: sender authorization paths” as the scope boundary: everything materially related to that caption should be covered by evidence (automated or documented) in this change, and anything intentionally excluded should be called out with rationale and residual risk.

**Issue caption:** pause_stream / resume_stream: sender authorization paths

### Domain context

This repository hosts the Fluxora streaming contract on Soroban (Stellar). Streams lock funded assets in contract
storage, release entitlement over time according to schedule parameters (including cliffs), and expose explicit roles:
senders who fund and may adjust certain parameters, recipients who claim vested value, administrators who may intervene
under policy, and—where applicable—permissionless callers for maintenance reads or cleanup. Integrators (wallets,
indexers, treasury tooling) depend on coherent state transitions, emitted signals, and error behavior when inputs or
timing are invalid.

### Work to complete

1. Characterize the **intended protocol semantics** for *pause_stream / resume_stream: sender authorization paths* in both success and failure cases, treating the **Summary** above as the authoritative scope statement.
2. Map which roles may participate, what each role must prove, and which operations must be impossible for non-authorized actors.
3. Enumerate edge cases driven by time (start, cliff, end, cancellation freeze), numeric ranges, and stream status combinations; capture expectations with tests or explicit audit exceptions.
4. Ensure externally visible behavior—persistent fields, contract errors, and emitted payloads—does not contradict documentation that integrators rely on.

### Definition of done

- An independent reader can confirm *pause_stream / resume_stream: sender authorization paths* matches the stated intent using tests, traces, snapshots (where used), or written review notes.
- Documentation for this slice of the protocol is consistent with observable on-chain behavior after the change.
- The pull request explains verification performed, scope boundaries, and any residual assumptions or risks.

### Constraints for contributors

- Describe **outcomes**, **invariants**, and **evidence**, not a single “right” internal design unless the issue title already names a concrete subsystem.
- Prefer **observable** guarantees: state transitions, balances, authorization failures, emitted events, error classifications, and documentation that integrators rely on.
- If something cannot be tested automatically, capture the gap as **audit notes** with explicit rationale and residual risk.

## Requirements and context

- Must be **secure**, **well-tested**, and **documented**.
- Should be **gas-conscious** (Soroban budget) and **easy to audit**.

## Suggested execution

1. Fork the repo and create a branch:
   ```bash
   git checkout -b feature/pause-resume-sender
   ```
2. Implement changes
   - **Contract / modules:** `contracts/stream/src/lib.rs`
   - **Comprehensive tests:** `contracts/stream/src/test.rs` and `contracts/stream/tests/integration_suite.rs`
   - **Documentation:** `docs/` (e.g. `streaming.md`, `security.md`, `events.md`)
   - Include **Rust doc comments** on public items (NatSpec-style clarity for auditors)
   - **Validate security assumptions** (CEI ordering, token trust model, auth paths)
3. **Test and commit**
   - Run `cargo test` in `contracts/stream` (and workspace if applicable)
   - Cover **edge cases** (overflow, TTL, paused/cancelled/completed)
   - Attach **test output** summary + **security notes** in the PR

### Example commit message

```
test(stream): cover sender pause/resume transitions
```

## Guidelines

- Aim for **≥95%** coverage on touched Rust modules (`cargo llvm-cov` or project-standard tooling).
- **Clear documentation** (protocol semantics + operator runbooks where relevant)
- **Timeframe:** 96 hours

++++++

---
type: Task
title: Admin overrides: pause/resume as admin
labels: contracts, soroban, admin
assignees: ''
---

## Description

### Summary

Within the Fluxora streaming contract, this work tightens externally visible assurances for one focused concern. Treasury operators, recipient-facing applications, and third-party auditors must be able to reason about this area using only on-chain observables and published protocol documentation—without inferring hidden rules from how the implementation happens to be structured. The scenarios captured by this issue title must therefore have crisp success semantics, crisp failure semantics, and no silent drift between stored state, user-visible errors, and emitted events. Time boundaries (start, cliff, end, cancellation freeze) and numeric ranges must be enumerated so entitlement cannot be misread under edge timing. In practical terms, treat “Admin overrides: pause/resume as admin” as the scope boundary: everything materially related to that caption should be covered by evidence (automated or documented) in this change, and anything intentionally excluded should be called out with rationale and residual risk.

**Issue caption:** Admin overrides: pause/resume as admin

### Domain context

This repository hosts the Fluxora streaming contract on Soroban (Stellar). Streams lock funded assets in contract
storage, release entitlement over time according to schedule parameters (including cliffs), and expose explicit roles:
senders who fund and may adjust certain parameters, recipients who claim vested value, administrators who may intervene
under policy, and—where applicable—permissionless callers for maintenance reads or cleanup. Integrators (wallets,
indexers, treasury tooling) depend on coherent state transitions, emitted signals, and error behavior when inputs or
timing are invalid.

### Work to complete

1. Characterize the **intended protocol semantics** for *Admin overrides: pause/resume as admin* in both success and failure cases, treating the **Summary** above as the authoritative scope statement.
2. Map which roles may participate, what each role must prove, and which operations must be impossible for non-authorized actors.
3. Enumerate edge cases driven by time (start, cliff, end, cancellation freeze), numeric ranges, and stream status combinations; capture expectations with tests or explicit audit exceptions.
4. Ensure externally visible behavior—persistent fields, contract errors, and emitted payloads—does not contradict documentation that integrators rely on.

### Definition of done

- An independent reader can confirm *Admin overrides: pause/resume as admin* matches the stated intent using tests, traces, snapshots (where used), or written review notes.
- Documentation for this slice of the protocol is consistent with observable on-chain behavior after the change.
- The pull request explains verification performed, scope boundaries, and any residual assumptions or risks.

### Constraints for contributors

- Describe **outcomes**, **invariants**, and **evidence**, not a single “right” internal design unless the issue title already names a concrete subsystem.
- Prefer **observable** guarantees: state transitions, balances, authorization failures, emitted events, error classifications, and documentation that integrators rely on.
- If something cannot be tested automatically, capture the gap as **audit notes** with explicit rationale and residual risk.

## Requirements and context

- Must be **secure**, **well-tested**, and **documented**.
- Should be **gas-conscious** (Soroban budget) and **easy to audit**.

## Suggested execution

1. Fork the repo and create a branch:
   ```bash
   git checkout -b feature/admin-pause-resume
   ```
2. Implement changes
   - **Contract / modules:** `contracts/stream/src/lib.rs`
   - **Comprehensive tests:** `contracts/stream/src/test.rs` and `contracts/stream/tests/integration_suite.rs`
   - **Documentation:** `docs/` (e.g. `streaming.md`, `security.md`, `events.md`)
   - Include **Rust doc comments** on public items (NatSpec-style clarity for auditors)
   - **Validate security assumptions** (CEI ordering, token trust model, auth paths)
3. **Test and commit**
   - Run `cargo test` in `contracts/stream` (and workspace if applicable)
   - Cover **edge cases** (overflow, TTL, paused/cancelled/completed)
   - Attach **test output** summary + **security notes** in the PR

### Example commit message

```
test(stream): cover admin pause/resume
```

## Guidelines

- Aim for **≥95%** coverage on touched Rust modules (`cargo llvm-cov` or project-standard tooling).
- **Clear documentation** (protocol semantics + operator runbooks where relevant)
- **Timeframe:** 96 hours

++++++

---
type: Feature
title: cancel_stream: refund and cancelled_at semantics
labels: contracts, soroban, lifecycle, tokens
assignees: ''
---

## Description

### Summary

Within the Fluxora streaming contract, this work tightens externally visible assurances for one focused concern. Treasury operators, recipient-facing applications, and third-party auditors must be able to reason about this area using only on-chain observables and published protocol documentation—without inferring hidden rules from how the implementation happens to be structured. The scenarios captured by this issue title must therefore have crisp success semantics, crisp failure semantics, and no silent drift between stored state, user-visible errors, and emitted events. Authorization boundaries for this slice must be explicit: who may trigger each path, and what proof they must supply. In practical terms, treat “cancel_stream: refund and cancelled_at semantics” as the scope boundary: everything materially related to that caption should be covered by evidence (automated or documented) in this change, and anything intentionally excluded should be called out with rationale and residual risk.

**Issue caption:** cancel_stream: refund and cancelled_at semantics

### Domain context

This repository hosts the Fluxora streaming contract on Soroban (Stellar). Streams lock funded assets in contract
storage, release entitlement over time according to schedule parameters (including cliffs), and expose explicit roles:
senders who fund and may adjust certain parameters, recipients who claim vested value, administrators who may intervene
under policy, and—where applicable—permissionless callers for maintenance reads or cleanup. Integrators (wallets,
indexers, treasury tooling) depend on coherent state transitions, emitted signals, and error behavior when inputs or
timing are invalid.

### Work to complete

1. Characterize the **intended protocol semantics** for *cancel_stream: refund and cancelled_at semantics* in both success and failure cases, treating the **Summary** above as the authoritative scope statement.
2. Map which roles may participate, what each role must prove, and which operations must be impossible for non-authorized actors.
3. Enumerate edge cases driven by time (start, cliff, end, cancellation freeze), numeric ranges, and stream status combinations; capture expectations with tests or explicit audit exceptions.
4. Ensure externally visible behavior—persistent fields, contract errors, and emitted payloads—does not contradict documentation that integrators rely on.

### Definition of done

- An independent reader can confirm *cancel_stream: refund and cancelled_at semantics* matches the stated intent using tests, traces, snapshots (where used), or written review notes.
- Documentation for this slice of the protocol is consistent with observable on-chain behavior after the change.
- The pull request explains verification performed, scope boundaries, and any residual assumptions or risks.

### Constraints for contributors

- Describe **outcomes**, **invariants**, and **evidence**, not a single “right” internal design unless the issue title already names a concrete subsystem.
- Prefer **observable** guarantees: state transitions, balances, authorization failures, emitted events, error classifications, and documentation that integrators rely on.
- If something cannot be tested automatically, capture the gap as **audit notes** with explicit rationale and residual risk.

## Requirements and context

- Must be **secure**, **well-tested**, and **documented**.
- Should be **gas-conscious** (Soroban budget) and **easy to audit**.

## Suggested execution

1. Fork the repo and create a branch:
   ```bash
   git checkout -b feature/cancel-stream-semantics
   ```
2. Implement changes
   - **Contract / modules:** `contracts/stream/src/lib.rs`
   - **Comprehensive tests:** `contracts/stream/src/test.rs` and `contracts/stream/tests/integration_suite.rs`
   - **Documentation:** `docs/` (e.g. `streaming.md`, `security.md`, `events.md`)
   - Include **Rust doc comments** on public items (NatSpec-style clarity for auditors)
   - **Validate security assumptions** (CEI ordering, token trust model, auth paths)
3. **Test and commit**
   - Run `cargo test` in `contracts/stream` (and workspace if applicable)
   - Cover **edge cases** (overflow, TTL, paused/cancelled/completed)
   - Attach **test output** summary + **security notes** in the PR

### Example commit message

```
test(stream): cover cancel refund and timestamp
```

## Guidelines

- Aim for **≥95%** coverage on touched Rust modules (`cargo llvm-cov` or project-standard tooling).
- **Clear documentation** (protocol semantics + operator runbooks where relevant)
- **Timeframe:** 96 hours

++++++

---
type: Feature
title: cancel_stream_as_admin: operational cancel parity
labels: contracts, soroban, admin
assignees: ''
---

## Description

### Summary

Within the Fluxora streaming contract, this work tightens externally visible assurances for one focused concern. Treasury operators, recipient-facing applications, and third-party auditors must be able to reason about this area using only on-chain observables and published protocol documentation—without inferring hidden rules from how the implementation happens to be structured. The scenarios captured by this issue title must therefore have crisp success semantics, crisp failure semantics, and no silent drift between stored state, user-visible errors, and emitted events. Authorization boundaries for this slice must be explicit: who may trigger each path, and what proof they must supply. In practical terms, treat “cancel_stream_as_admin: operational cancel parity” as the scope boundary: everything materially related to that caption should be covered by evidence (automated or documented) in this change, and anything intentionally excluded should be called out with rationale and residual risk.

**Issue caption:** cancel_stream_as_admin: operational cancel parity

### Domain context

This repository hosts the Fluxora streaming contract on Soroban (Stellar). Streams lock funded assets in contract
storage, release entitlement over time according to schedule parameters (including cliffs), and expose explicit roles:
senders who fund and may adjust certain parameters, recipients who claim vested value, administrators who may intervene
under policy, and—where applicable—permissionless callers for maintenance reads or cleanup. Integrators (wallets,
indexers, treasury tooling) depend on coherent state transitions, emitted signals, and error behavior when inputs or
timing are invalid.

### Work to complete

1. Characterize the **intended protocol semantics** for *cancel_stream_as_admin: operational cancel parity* in both success and failure cases, treating the **Summary** above as the authoritative scope statement.
2. Map which roles may participate, what each role must prove, and which operations must be impossible for non-authorized actors.
3. Enumerate edge cases driven by time (start, cliff, end, cancellation freeze), numeric ranges, and stream status combinations; capture expectations with tests or explicit audit exceptions.
4. Ensure externally visible behavior—persistent fields, contract errors, and emitted payloads—does not contradict documentation that integrators rely on.

### Definition of done

- An independent reader can confirm *cancel_stream_as_admin: operational cancel parity* matches the stated intent using tests, traces, snapshots (where used), or written review notes.
- Documentation for this slice of the protocol is consistent with observable on-chain behavior after the change.
- The pull request explains verification performed, scope boundaries, and any residual assumptions or risks.

### Constraints for contributors

- Describe **outcomes**, **invariants**, and **evidence**, not a single “right” internal design unless the issue title already names a concrete subsystem.
- Prefer **observable** guarantees: state transitions, balances, authorization failures, emitted events, error classifications, and documentation that integrators rely on.
- If something cannot be tested automatically, capture the gap as **audit notes** with explicit rationale and residual risk.

## Requirements and context

- Must be **secure**, **well-tested**, and **documented**.
- Should be **gas-conscious** (Soroban budget) and **easy to audit**.

## Suggested execution

1. Fork the repo and create a branch:
   ```bash
   git checkout -b feature/cancel-admin-parity
   ```
2. Implement changes
   - **Contract / modules:** `contracts/stream/src/lib.rs`
   - **Comprehensive tests:** `contracts/stream/src/test.rs` and `contracts/stream/tests/integration_suite.rs`
   - **Documentation:** `docs/` (e.g. `streaming.md`, `security.md`, `events.md`)
   - Include **Rust doc comments** on public items (NatSpec-style clarity for auditors)
   - **Validate security assumptions** (CEI ordering, token trust model, auth paths)
3. **Test and commit**
   - Run `cargo test` in `contracts/stream` (and workspace if applicable)
   - Cover **edge cases** (overflow, TTL, paused/cancelled/completed)
   - Attach **test output** summary + **security notes** in the PR

### Example commit message

```
test(stream): cover admin cancel parity
```

## Guidelines

- Aim for **≥95%** coverage on touched Rust modules (`cargo llvm-cov` or project-standard tooling).
- **Clear documentation** (protocol semantics + operator runbooks where relevant)
- **Timeframe:** 96 hours

++++++

---
type: Feature
title: CEI ordering: external token calls after state updates
labels: contracts, soroban, security
assignees: ''
---

## Description

### Summary

Within the Fluxora streaming contract, this work tightens externally visible assurances for one focused concern. Treasury operators, recipient-facing applications, and third-party auditors must be able to reason about this area using only on-chain observables and published protocol documentation—without inferring hidden rules from how the implementation happens to be structured. The scenarios captured by this issue title must therefore have crisp success semantics, crisp failure semantics, and no silent drift between stored state, user-visible errors, and emitted events. Time boundaries (start, cliff, end, cancellation freeze) and numeric ranges must be enumerated so entitlement cannot be misread under edge timing. In practical terms, treat “CEI ordering: external token calls after state updates” as the scope boundary: everything materially related to that caption should be covered by evidence (automated or documented) in this change, and anything intentionally excluded should be called out with rationale and residual risk.

**Issue caption:** CEI ordering: external token calls after state updates

### Domain context

This repository hosts the Fluxora streaming contract on Soroban (Stellar). Streams lock funded assets in contract
storage, release entitlement over time according to schedule parameters (including cliffs), and expose explicit roles:
senders who fund and may adjust certain parameters, recipients who claim vested value, administrators who may intervene
under policy, and—where applicable—permissionless callers for maintenance reads or cleanup. Integrators (wallets,
indexers, treasury tooling) depend on coherent state transitions, emitted signals, and error behavior when inputs or
timing are invalid.

### Work to complete

1. Characterize the **intended protocol semantics** for *CEI ordering: external token calls after state updates* in both success and failure cases, treating the **Summary** above as the authoritative scope statement.
2. Map which roles may participate, what each role must prove, and which operations must be impossible for non-authorized actors.
3. Enumerate edge cases driven by time (start, cliff, end, cancellation freeze), numeric ranges, and stream status combinations; capture expectations with tests or explicit audit exceptions.
4. Ensure externally visible behavior—persistent fields, contract errors, and emitted payloads—does not contradict documentation that integrators rely on.

### Definition of done

- An independent reader can confirm *CEI ordering: external token calls after state updates* matches the stated intent using tests, traces, snapshots (where used), or written review notes.
- Documentation for this slice of the protocol is consistent with observable on-chain behavior after the change.
- The pull request explains verification performed, scope boundaries, and any residual assumptions or risks.

### Constraints for contributors

- Describe **outcomes**, **invariants**, and **evidence**, not a single “right” internal design unless the issue title already names a concrete subsystem.
- Prefer **observable** guarantees: state transitions, balances, authorization failures, emitted events, error classifications, and documentation that integrators rely on.
- If something cannot be tested automatically, capture the gap as **audit notes** with explicit rationale and residual risk.

## Requirements and context

- Must be **secure**, **well-tested**, and **documented**.
- Should be **gas-conscious** (Soroban budget) and **easy to audit**.

## Suggested execution

1. Fork the repo and create a branch:
   ```bash
   git checkout -b feature/cei-audit
   ```
2. Implement changes
   - **Contract / modules:** `contracts/stream/src/lib.rs`
   - **Comprehensive tests:** `contracts/stream/src/test.rs` and `contracts/stream/tests/integration_suite.rs`
   - **Documentation:** `docs/` (e.g. `streaming.md`, `security.md`, `events.md`)
   - Include **Rust doc comments** on public items (NatSpec-style clarity for auditors)
   - **Validate security assumptions** (CEI ordering, token trust model, auth paths)
3. **Test and commit**
   - Run `cargo test` in `contracts/stream` (and workspace if applicable)
   - Cover **edge cases** (overflow, TTL, paused/cancelled/completed)
   - Attach **test output** summary + **security notes** in the PR

### Example commit message

```
docs(stream): document CEI and token trust assumptions
```

## Guidelines

- Aim for **≥95%** coverage on touched Rust modules (`cargo llvm-cov` or project-standard tooling).
- **Clear documentation** (protocol semantics + operator runbooks where relevant)
- **Timeframe:** 96 hours

++++++

---
type: Feature
title: withdraw: recipient-only auth and completion transition
labels: contracts, soroban, tokens
assignees: ''
---

## Description

### Summary

Within the Fluxora streaming contract, this work tightens externally visible assurances for one focused concern. Treasury operators, recipient-facing applications, and third-party auditors must be able to reason about this area using only on-chain observables and published protocol documentation—without inferring hidden rules from how the implementation happens to be structured. The scenarios captured by this issue title must therefore have crisp success semantics, crisp failure semantics, and no silent drift between stored state, user-visible errors, and emitted events. Emitted signals and error classifications must remain coherent for indexers and wallets building on this protocol surface. In practical terms, treat “withdraw: recipient-only auth and completion transition” as the scope boundary: everything materially related to that caption should be covered by evidence (automated or documented) in this change, and anything intentionally excluded should be called out with rationale and residual risk.

**Issue caption:** withdraw: recipient-only auth and completion transition

### Domain context

This repository hosts the Fluxora streaming contract on Soroban (Stellar). Streams lock funded assets in contract
storage, release entitlement over time according to schedule parameters (including cliffs), and expose explicit roles:
senders who fund and may adjust certain parameters, recipients who claim vested value, administrators who may intervene
under policy, and—where applicable—permissionless callers for maintenance reads or cleanup. Integrators (wallets,
indexers, treasury tooling) depend on coherent state transitions, emitted signals, and error behavior when inputs or
timing are invalid.

### Work to complete

1. Characterize the **intended protocol semantics** for *withdraw: recipient-only auth and completion transition* in both success and failure cases, treating the **Summary** above as the authoritative scope statement.
2. Map which roles may participate, what each role must prove, and which operations must be impossible for non-authorized actors.
3. Enumerate edge cases driven by time (start, cliff, end, cancellation freeze), numeric ranges, and stream status combinations; capture expectations with tests or explicit audit exceptions.
4. Ensure externally visible behavior—persistent fields, contract errors, and emitted payloads—does not contradict documentation that integrators rely on.

### Definition of done

- An independent reader can confirm *withdraw: recipient-only auth and completion transition* matches the stated intent using tests, traces, snapshots (where used), or written review notes.
- Documentation for this slice of the protocol is consistent with observable on-chain behavior after the change.
- The pull request explains verification performed, scope boundaries, and any residual assumptions or risks.

### Constraints for contributors

- Describe **outcomes**, **invariants**, and **evidence**, not a single “right” internal design unless the issue title already names a concrete subsystem.
- Prefer **observable** guarantees: state transitions, balances, authorization failures, emitted events, error classifications, and documentation that integrators rely on.
- If something cannot be tested automatically, capture the gap as **audit notes** with explicit rationale and residual risk.

## Requirements and context

- Must be **secure**, **well-tested**, and **documented**.
- Should be **gas-conscious** (Soroban budget) and **easy to audit**.

## Suggested execution

1. Fork the repo and create a branch:
   ```bash
   git checkout -b feature/withdraw-core
   ```
2. Implement changes
   - **Contract / modules:** `contracts/stream/src/lib.rs`
   - **Comprehensive tests:** `contracts/stream/src/test.rs` and `contracts/stream/tests/integration_suite.rs`
   - **Documentation:** `docs/` (e.g. `streaming.md`, `security.md`, `events.md`)
   - Include **Rust doc comments** on public items (NatSpec-style clarity for auditors)
   - **Validate security assumptions** (CEI ordering, token trust model, auth paths)
3. **Test and commit**
   - Run `cargo test` in `contracts/stream` (and workspace if applicable)
   - Cover **edge cases** (overflow, TTL, paused/cancelled/completed)
   - Attach **test output** summary + **security notes** in the PR

### Example commit message

```
test(stream): cover withdraw completion transitions
```

## Guidelines

- Aim for **≥95%** coverage on touched Rust modules (`cargo llvm-cov` or project-standard tooling).
- **Clear documentation** (protocol semantics + operator runbooks where relevant)
- **Timeframe:** 96 hours

++++++

---
type: Task
title: withdraw: zero-withdrawable idempotency (no state churn)
labels: contracts, soroban, ux
assignees: ''
---

## Description

### Summary

Within the Fluxora streaming contract, this work tightens externally visible assurances for one focused concern. Treasury operators, recipient-facing applications, and third-party auditors must be able to reason about this area using only on-chain observables and published protocol documentation—without inferring hidden rules from how the implementation happens to be structured. The scenarios captured by this issue title must therefore have crisp success semantics, crisp failure semantics, and no silent drift between stored state, user-visible errors, and emitted events. Time boundaries (start, cliff, end, cancellation freeze) and numeric ranges must be enumerated so entitlement cannot be misread under edge timing. In practical terms, treat “withdraw: zero-withdrawable idempotency (no state churn)” as the scope boundary: everything materially related to that caption should be covered by evidence (automated or documented) in this change, and anything intentionally excluded should be called out with rationale and residual risk.

**Issue caption:** withdraw: zero-withdrawable idempotency (no state churn)

### Domain context

This repository hosts the Fluxora streaming contract on Soroban (Stellar). Streams lock funded assets in contract
storage, release entitlement over time according to schedule parameters (including cliffs), and expose explicit roles:
senders who fund and may adjust certain parameters, recipients who claim vested value, administrators who may intervene
under policy, and—where applicable—permissionless callers for maintenance reads or cleanup. Integrators (wallets,
indexers, treasury tooling) depend on coherent state transitions, emitted signals, and error behavior when inputs or
timing are invalid.

### Work to complete

1. Characterize the **intended protocol semantics** for *withdraw: zero-withdrawable idempotency (no state churn)* in both success and failure cases, treating the **Summary** above as the authoritative scope statement.
2. Map which roles may participate, what each role must prove, and which operations must be impossible for non-authorized actors.
3. Enumerate edge cases driven by time (start, cliff, end, cancellation freeze), numeric ranges, and stream status combinations; capture expectations with tests or explicit audit exceptions.
4. Ensure externally visible behavior—persistent fields, contract errors, and emitted payloads—does not contradict documentation that integrators rely on.

### Definition of done

- An independent reader can confirm *withdraw: zero-withdrawable idempotency (no state churn)* matches the stated intent using tests, traces, snapshots (where used), or written review notes.
- Documentation for this slice of the protocol is consistent with observable on-chain behavior after the change.
- The pull request explains verification performed, scope boundaries, and any residual assumptions or risks.

### Constraints for contributors

- Describe **outcomes**, **invariants**, and **evidence**, not a single “right” internal design unless the issue title already names a concrete subsystem.
- Prefer **observable** guarantees: state transitions, balances, authorization failures, emitted events, error classifications, and documentation that integrators rely on.
- If something cannot be tested automatically, capture the gap as **audit notes** with explicit rationale and residual risk.

## Requirements and context

- Must be **secure**, **well-tested**, and **documented**.
- Should be **gas-conscious** (Soroban budget) and **easy to audit**.

## Suggested execution

1. Fork the repo and create a branch:
   ```bash
   git checkout -b feature/withdraw-idempotent-zero
   ```
2. Implement changes
   - **Contract / modules:** `contracts/stream/src/lib.rs`
   - **Comprehensive tests:** `contracts/stream/src/test.rs` and `contracts/stream/tests/integration_suite.rs`
   - **Documentation:** `docs/` (e.g. `streaming.md`, `security.md`, `events.md`)
   - Include **Rust doc comments** on public items (NatSpec-style clarity for auditors)
   - **Validate security assumptions** (CEI ordering, token trust model, auth paths)
3. **Test and commit**
   - Run `cargo test` in `contracts/stream` (and workspace if applicable)
   - Cover **edge cases** (overflow, TTL, paused/cancelled/completed)
   - Attach **test output** summary + **security notes** in the PR

### Example commit message

```
test(stream): withdraw idempotent on zero
```

## Guidelines

- Aim for **≥95%** coverage on touched Rust modules (`cargo llvm-cov` or project-standard tooling).
- **Clear documentation** (protocol semantics + operator runbooks where relevant)
- **Timeframe:** 96 hours

++++++

---
type: Feature
title: withdraw_to: destination constraints and event parity
labels: contracts, soroban, custody
assignees: ''
---

## Description

### Summary

Within the Fluxora streaming contract, this work tightens externally visible assurances for one focused concern. Treasury operators, recipient-facing applications, and third-party auditors must be able to reason about this area using only on-chain observables and published protocol documentation—without inferring hidden rules from how the implementation happens to be structured. The scenarios captured by this issue title must therefore have crisp success semantics, crisp failure semantics, and no silent drift between stored state, user-visible errors, and emitted events. Permissionless maintenance or read paths, if involved, must have clearly documented trust impact for operators. In practical terms, treat “withdraw_to: destination constraints and event parity” as the scope boundary: everything materially related to that caption should be covered by evidence (automated or documented) in this change, and anything intentionally excluded should be called out with rationale and residual risk.

**Issue caption:** withdraw_to: destination constraints and event parity

### Domain context

This repository hosts the Fluxora streaming contract on Soroban (Stellar). Streams lock funded assets in contract
storage, release entitlement over time according to schedule parameters (including cliffs), and expose explicit roles:
senders who fund and may adjust certain parameters, recipients who claim vested value, administrators who may intervene
under policy, and—where applicable—permissionless callers for maintenance reads or cleanup. Integrators (wallets,
indexers, treasury tooling) depend on coherent state transitions, emitted signals, and error behavior when inputs or
timing are invalid.

### Work to complete

1. Characterize the **intended protocol semantics** for *withdraw_to: destination constraints and event parity* in both success and failure cases, treating the **Summary** above as the authoritative scope statement.
2. Map which roles may participate, what each role must prove, and which operations must be impossible for non-authorized actors.
3. Enumerate edge cases driven by time (start, cliff, end, cancellation freeze), numeric ranges, and stream status combinations; capture expectations with tests or explicit audit exceptions.
4. Ensure externally visible behavior—persistent fields, contract errors, and emitted payloads—does not contradict documentation that integrators rely on.

### Definition of done

- An independent reader can confirm *withdraw_to: destination constraints and event parity* matches the stated intent using tests, traces, snapshots (where used), or written review notes.
- Documentation for this slice of the protocol is consistent with observable on-chain behavior after the change.
- The pull request explains verification performed, scope boundaries, and any residual assumptions or risks.

### Constraints for contributors

- Describe **outcomes**, **invariants**, and **evidence**, not a single “right” internal design unless the issue title already names a concrete subsystem.
- Prefer **observable** guarantees: state transitions, balances, authorization failures, emitted events, error classifications, and documentation that integrators rely on.
- If something cannot be tested automatically, capture the gap as **audit notes** with explicit rationale and residual risk.

## Requirements and context

- Must be **secure**, **well-tested**, and **documented**.
- Should be **gas-conscious** (Soroban budget) and **easy to audit**.

## Suggested execution

1. Fork the repo and create a branch:
   ```bash
   git checkout -b feature/withdraw-to
   ```
2. Implement changes
   - **Contract / modules:** `contracts/stream/src/lib.rs`
   - **Comprehensive tests:** `contracts/stream/src/test.rs` and `contracts/stream/tests/integration_suite.rs`
   - **Documentation:** `docs/` (e.g. `streaming.md`, `security.md`, `events.md`)
   - Include **Rust doc comments** on public items (NatSpec-style clarity for auditors)
   - **Validate security assumptions** (CEI ordering, token trust model, auth paths)
3. **Test and commit**
   - Run `cargo test` in `contracts/stream` (and workspace if applicable)
   - Cover **edge cases** (overflow, TTL, paused/cancelled/completed)
   - Attach **test output** summary + **security notes** in the PR

### Example commit message

```
test(stream): cover withdraw_to destinations
```

## Guidelines

- Aim for **≥95%** coverage on touched Rust modules (`cargo llvm-cov` or project-standard tooling).
- **Clear documentation** (protocol semantics + operator runbooks where relevant)
- **Timeframe:** 96 hours

++++++

---
type: Feature
title: batch_withdraw: atomic failure semantics
labels: contracts, soroban, batch
assignees: ''
---

## Description

### Summary

Within the Fluxora streaming contract, this work tightens externally visible assurances for one focused concern. Treasury operators, recipient-facing applications, and third-party auditors must be able to reason about this area using only on-chain observables and published protocol documentation—without inferring hidden rules from how the implementation happens to be structured. The scenarios captured by this issue title must therefore have crisp success semantics, crisp failure semantics, and no silent drift between stored state, user-visible errors, and emitted events. Permissionless maintenance or read paths, if involved, must have clearly documented trust impact for operators. In practical terms, treat “batch_withdraw: atomic failure semantics” as the scope boundary: everything materially related to that caption should be covered by evidence (automated or documented) in this change, and anything intentionally excluded should be called out with rationale and residual risk.

**Issue caption:** batch_withdraw: atomic failure semantics

### Domain context

This repository hosts the Fluxora streaming contract on Soroban (Stellar). Streams lock funded assets in contract
storage, release entitlement over time according to schedule parameters (including cliffs), and expose explicit roles:
senders who fund and may adjust certain parameters, recipients who claim vested value, administrators who may intervene
under policy, and—where applicable—permissionless callers for maintenance reads or cleanup. Integrators (wallets,
indexers, treasury tooling) depend on coherent state transitions, emitted signals, and error behavior when inputs or
timing are invalid.

### Work to complete

1. Characterize the **intended protocol semantics** for *batch_withdraw: atomic failure semantics* in both success and failure cases, treating the **Summary** above as the authoritative scope statement.
2. Map which roles may participate, what each role must prove, and which operations must be impossible for non-authorized actors.
3. Enumerate edge cases driven by time (start, cliff, end, cancellation freeze), numeric ranges, and stream status combinations; capture expectations with tests or explicit audit exceptions.
4. Ensure externally visible behavior—persistent fields, contract errors, and emitted payloads—does not contradict documentation that integrators rely on.

### Definition of done

- An independent reader can confirm *batch_withdraw: atomic failure semantics* matches the stated intent using tests, traces, snapshots (where used), or written review notes.
- Documentation for this slice of the protocol is consistent with observable on-chain behavior after the change.
- The pull request explains verification performed, scope boundaries, and any residual assumptions or risks.

### Constraints for contributors

- Describe **outcomes**, **invariants**, and **evidence**, not a single “right” internal design unless the issue title already names a concrete subsystem.
- Prefer **observable** guarantees: state transitions, balances, authorization failures, emitted events, error classifications, and documentation that integrators rely on.
- If something cannot be tested automatically, capture the gap as **audit notes** with explicit rationale and residual risk.

## Requirements and context

- Must be **secure**, **well-tested**, and **documented**.
- Should be **gas-conscious** (Soroban budget) and **easy to audit**.

## Suggested execution

1. Fork the repo and create a branch:
   ```bash
   git checkout -b feature/batch-withdraw-atomic
   ```
2. Implement changes
   - **Contract / modules:** `contracts/stream/src/lib.rs`
   - **Comprehensive tests:** `contracts/stream/src/test.rs` and `contracts/stream/tests/integration_suite.rs`
   - **Documentation:** `docs/` (e.g. `streaming.md`, `security.md`, `events.md`)
   - Include **Rust doc comments** on public items (NatSpec-style clarity for auditors)
   - **Validate security assumptions** (CEI ordering, token trust model, auth paths)
3. **Test and commit**
   - Run `cargo test` in `contracts/stream` (and workspace if applicable)
   - Cover **edge cases** (overflow, TTL, paused/cancelled/completed)
   - Attach **test output** summary + **security notes** in the PR

### Example commit message

```
test(stream): harden batch_withdraw atomicity
```

## Guidelines

- Aim for **≥95%** coverage on touched Rust modules (`cargo llvm-cov` or project-standard tooling).
- **Clear documentation** (protocol semantics + operator runbooks where relevant)
- **Timeframe:** 96 hours

++++++

---
type: Feature
title: batch_withdraw: completed streams yield zero amounts
labels: contracts, soroban, batch
assignees: ''
---

## Description

### Summary

Within the Fluxora streaming contract, this work tightens externally visible assurances for one focused concern. Treasury operators, recipient-facing applications, and third-party auditors must be able to reason about this area using only on-chain observables and published protocol documentation—without inferring hidden rules from how the implementation happens to be structured. The scenarios captured by this issue title must therefore have crisp success semantics, crisp failure semantics, and no silent drift between stored state, user-visible errors, and emitted events. Economic conservation and payout ordering assumptions must be defensible to a treasury or auditor without reference to internal layout. In practical terms, treat “batch_withdraw: completed streams yield zero amounts” as the scope boundary: everything materially related to that caption should be covered by evidence (automated or documented) in this change, and anything intentionally excluded should be called out with rationale and residual risk.

**Issue caption:** batch_withdraw: completed streams yield zero amounts

### Domain context

This repository hosts the Fluxora streaming contract on Soroban (Stellar). Streams lock funded assets in contract
storage, release entitlement over time according to schedule parameters (including cliffs), and expose explicit roles:
senders who fund and may adjust certain parameters, recipients who claim vested value, administrators who may intervene
under policy, and—where applicable—permissionless callers for maintenance reads or cleanup. Integrators (wallets,
indexers, treasury tooling) depend on coherent state transitions, emitted signals, and error behavior when inputs or
timing are invalid.

### Work to complete

1. Characterize the **intended protocol semantics** for *batch_withdraw: completed streams yield zero amounts* in both success and failure cases, treating the **Summary** above as the authoritative scope statement.
2. Map which roles may participate, what each role must prove, and which operations must be impossible for non-authorized actors.
3. Enumerate edge cases driven by time (start, cliff, end, cancellation freeze), numeric ranges, and stream status combinations; capture expectations with tests or explicit audit exceptions.
4. Ensure externally visible behavior—persistent fields, contract errors, and emitted payloads—does not contradict documentation that integrators rely on.

### Definition of done

- An independent reader can confirm *batch_withdraw: completed streams yield zero amounts* matches the stated intent using tests, traces, snapshots (where used), or written review notes.
- Documentation for this slice of the protocol is consistent with observable on-chain behavior after the change.
- The pull request explains verification performed, scope boundaries, and any residual assumptions or risks.

### Constraints for contributors

- Describe **outcomes**, **invariants**, and **evidence**, not a single “right” internal design unless the issue title already names a concrete subsystem.
- Prefer **observable** guarantees: state transitions, balances, authorization failures, emitted events, error classifications, and documentation that integrators rely on.
- If something cannot be tested automatically, capture the gap as **audit notes** with explicit rationale and residual risk.

## Requirements and context

- Must be **secure**, **well-tested**, and **documented**.
- Should be **gas-conscious** (Soroban budget) and **easy to audit**.

## Suggested execution

1. Fork the repo and create a branch:
   ```bash
   git checkout -b feature/batch-withdraw-completed
   ```
2. Implement changes
   - **Contract / modules:** `contracts/stream/src/lib.rs`
   - **Comprehensive tests:** `contracts/stream/src/test.rs` and `contracts/stream/tests/integration_suite.rs`
   - **Documentation:** `docs/` (e.g. `streaming.md`, `security.md`, `events.md`)
   - Include **Rust doc comments** on public items (NatSpec-style clarity for auditors)
   - **Validate security assumptions** (CEI ordering, token trust model, auth paths)
3. **Test and commit**
   - Run `cargo test` in `contracts/stream` (and workspace if applicable)
   - Cover **edge cases** (overflow, TTL, paused/cancelled/completed)
   - Attach **test output** summary + **security notes** in the PR

### Example commit message

```
test(stream): batch withdraw with completed streams
```

## Guidelines

- Aim for **≥95%** coverage on touched Rust modules (`cargo llvm-cov` or project-standard tooling).
- **Clear documentation** (protocol semantics + operator runbooks where relevant)
- **Timeframe:** 96 hours

++++++

---
type: Feature
title: calculate_accrued: status-specific behavior matrix
labels: contracts, soroban, math, views
assignees: ''
---

## Description

### Summary

Within the Fluxora streaming contract, this work tightens externally visible assurances for one focused concern. Treasury operators, recipient-facing applications, and third-party auditors must be able to reason about this area using only on-chain observables and published protocol documentation—without inferring hidden rules from how the implementation happens to be structured. The scenarios captured by this issue title must therefore have crisp success semantics, crisp failure semantics, and no silent drift between stored state, user-visible errors, and emitted events. Emitted signals and error classifications must remain coherent for indexers and wallets building on this protocol surface. In practical terms, treat “calculate_accrued: status-specific behavior matrix” as the scope boundary: everything materially related to that caption should be covered by evidence (automated or documented) in this change, and anything intentionally excluded should be called out with rationale and residual risk.

**Issue caption:** calculate_accrued: status-specific behavior matrix

### Domain context

This repository hosts the Fluxora streaming contract on Soroban (Stellar). Streams lock funded assets in contract
storage, release entitlement over time according to schedule parameters (including cliffs), and expose explicit roles:
senders who fund and may adjust certain parameters, recipients who claim vested value, administrators who may intervene
under policy, and—where applicable—permissionless callers for maintenance reads or cleanup. Integrators (wallets,
indexers, treasury tooling) depend on coherent state transitions, emitted signals, and error behavior when inputs or
timing are invalid.

### Work to complete

1. Characterize the **intended protocol semantics** for *calculate_accrued: status-specific behavior matrix* in both success and failure cases, treating the **Summary** above as the authoritative scope statement.
2. Map which roles may participate, what each role must prove, and which operations must be impossible for non-authorized actors.
3. Enumerate edge cases driven by time (start, cliff, end, cancellation freeze), numeric ranges, and stream status combinations; capture expectations with tests or explicit audit exceptions.
4. Ensure externally visible behavior—persistent fields, contract errors, and emitted payloads—does not contradict documentation that integrators rely on.

### Definition of done

- An independent reader can confirm *calculate_accrued: status-specific behavior matrix* matches the stated intent using tests, traces, snapshots (where used), or written review notes.
- Documentation for this slice of the protocol is consistent with observable on-chain behavior after the change.
- The pull request explains verification performed, scope boundaries, and any residual assumptions or risks.

### Constraints for contributors

- Describe **outcomes**, **invariants**, and **evidence**, not a single “right” internal design unless the issue title already names a concrete subsystem.
- Prefer **observable** guarantees: state transitions, balances, authorization failures, emitted events, error classifications, and documentation that integrators rely on.
- If something cannot be tested automatically, capture the gap as **audit notes** with explicit rationale and residual risk.

## Requirements and context

- Must be **secure**, **well-tested**, and **documented**.
- Should be **gas-conscious** (Soroban budget) and **easy to audit**.

## Suggested execution

1. Fork the repo and create a branch:
   ```bash
   git checkout -b feature/calculate-accrued-matrix
   ```
2. Implement changes
   - **Contract / modules:** `contracts/stream/src/lib.rs`
   - **Comprehensive tests:** `contracts/stream/src/test.rs` and `contracts/stream/tests/integration_suite.rs`
   - **Documentation:** `docs/` (e.g. `streaming.md`, `security.md`, `events.md`)
   - Include **Rust doc comments** on public items (NatSpec-style clarity for auditors)
   - **Validate security assumptions** (CEI ordering, token trust model, auth paths)
3. **Test and commit**
   - Run `cargo test` in `contracts/stream` (and workspace if applicable)
   - Cover **edge cases** (overflow, TTL, paused/cancelled/completed)
   - Attach **test output** summary + **security notes** in the PR

### Example commit message

```
test(stream): calculate_accrued status matrix
```

## Guidelines

- Aim for **≥95%** coverage on touched Rust modules (`cargo llvm-cov` or project-standard tooling).
- **Clear documentation** (protocol semantics + operator runbooks where relevant)
- **Timeframe:** 96 hours

++++++

---
type: Task
title: get_withdrawable vs withdraw consistency
labels: contracts, soroban, views
assignees: ''
---

## Description

### Summary

Within the Fluxora streaming contract, this work tightens externally visible assurances for one focused concern. Treasury operators, recipient-facing applications, and third-party auditors must be able to reason about this area using only on-chain observables and published protocol documentation—without inferring hidden rules from how the implementation happens to be structured. The scenarios captured by this issue title must therefore have crisp success semantics, crisp failure semantics, and no silent drift between stored state, user-visible errors, and emitted events. Time boundaries (start, cliff, end, cancellation freeze) and numeric ranges must be enumerated so entitlement cannot be misread under edge timing. In practical terms, treat “get_withdrawable vs withdraw consistency” as the scope boundary: everything materially related to that caption should be covered by evidence (automated or documented) in this change, and anything intentionally excluded should be called out with rationale and residual risk.

**Issue caption:** get_withdrawable vs withdraw consistency

### Domain context

This repository hosts the Fluxora streaming contract on Soroban (Stellar). Streams lock funded assets in contract
storage, release entitlement over time according to schedule parameters (including cliffs), and expose explicit roles:
senders who fund and may adjust certain parameters, recipients who claim vested value, administrators who may intervene
under policy, and—where applicable—permissionless callers for maintenance reads or cleanup. Integrators (wallets,
indexers, treasury tooling) depend on coherent state transitions, emitted signals, and error behavior when inputs or
timing are invalid.

### Work to complete

1. Characterize the **intended protocol semantics** for *get_withdrawable vs withdraw consistency* in both success and failure cases, treating the **Summary** above as the authoritative scope statement.
2. Map which roles may participate, what each role must prove, and which operations must be impossible for non-authorized actors.
3. Enumerate edge cases driven by time (start, cliff, end, cancellation freeze), numeric ranges, and stream status combinations; capture expectations with tests or explicit audit exceptions.
4. Ensure externally visible behavior—persistent fields, contract errors, and emitted payloads—does not contradict documentation that integrators rely on.

### Definition of done

- An independent reader can confirm *get_withdrawable vs withdraw consistency* matches the stated intent using tests, traces, snapshots (where used), or written review notes.
- Documentation for this slice of the protocol is consistent with observable on-chain behavior after the change.
- The pull request explains verification performed, scope boundaries, and any residual assumptions or risks.

### Constraints for contributors

- Describe **outcomes**, **invariants**, and **evidence**, not a single “right” internal design unless the issue title already names a concrete subsystem.
- Prefer **observable** guarantees: state transitions, balances, authorization failures, emitted events, error classifications, and documentation that integrators rely on.
- If something cannot be tested automatically, capture the gap as **audit notes** with explicit rationale and residual risk.

## Requirements and context

- Must be **secure**, **well-tested**, and **documented**.
- Should be **gas-conscious** (Soroban budget) and **easy to audit**.

## Suggested execution

1. Fork the repo and create a branch:
   ```bash
   git checkout -b feature/get-withdrawable-parity
   ```
2. Implement changes
   - **Contract / modules:** `contracts/stream/src/lib.rs`
   - **Comprehensive tests:** `contracts/stream/src/test.rs` and `contracts/stream/tests/integration_suite.rs`
   - **Documentation:** `docs/` (e.g. `streaming.md`, `security.md`, `events.md`)
   - Include **Rust doc comments** on public items (NatSpec-style clarity for auditors)
   - **Validate security assumptions** (CEI ordering, token trust model, auth paths)
3. **Test and commit**
   - Run `cargo test` in `contracts/stream` (and workspace if applicable)
   - Cover **edge cases** (overflow, TTL, paused/cancelled/completed)
   - Attach **test output** summary + **security notes** in the PR

### Example commit message

```
test(stream): align get_withdrawable with withdraw
```

## Guidelines

- Aim for **≥95%** coverage on touched Rust modules (`cargo llvm-cov` or project-standard tooling).
- **Clear documentation** (protocol semantics + operator runbooks where relevant)
- **Timeframe:** 96 hours

++++++

---
type: Feature
title: get_claimable_at: future simulation and cancel clamping
labels: contracts, soroban, views
assignees: ''
---

## Description

### Summary

Within the Fluxora streaming contract, this work tightens externally visible assurances for one focused concern. Treasury operators, recipient-facing applications, and third-party auditors must be able to reason about this area using only on-chain observables and published protocol documentation—without inferring hidden rules from how the implementation happens to be structured. The scenarios captured by this issue title must therefore have crisp success semantics, crisp failure semantics, and no silent drift between stored state, user-visible errors, and emitted events. Permissionless maintenance or read paths, if involved, must have clearly documented trust impact for operators. In practical terms, treat “get_claimable_at: future simulation and cancel clamping” as the scope boundary: everything materially related to that caption should be covered by evidence (automated or documented) in this change, and anything intentionally excluded should be called out with rationale and residual risk.

**Issue caption:** get_claimable_at: future simulation and cancel clamping

### Domain context

This repository hosts the Fluxora streaming contract on Soroban (Stellar). Streams lock funded assets in contract
storage, release entitlement over time according to schedule parameters (including cliffs), and expose explicit roles:
senders who fund and may adjust certain parameters, recipients who claim vested value, administrators who may intervene
under policy, and—where applicable—permissionless callers for maintenance reads or cleanup. Integrators (wallets,
indexers, treasury tooling) depend on coherent state transitions, emitted signals, and error behavior when inputs or
timing are invalid.

### Work to complete

1. Characterize the **intended protocol semantics** for *get_claimable_at: future simulation and cancel clamping* in both success and failure cases, treating the **Summary** above as the authoritative scope statement.
2. Map which roles may participate, what each role must prove, and which operations must be impossible for non-authorized actors.
3. Enumerate edge cases driven by time (start, cliff, end, cancellation freeze), numeric ranges, and stream status combinations; capture expectations with tests or explicit audit exceptions.
4. Ensure externally visible behavior—persistent fields, contract errors, and emitted payloads—does not contradict documentation that integrators rely on.

### Definition of done

- An independent reader can confirm *get_claimable_at: future simulation and cancel clamping* matches the stated intent using tests, traces, snapshots (where used), or written review notes.
- Documentation for this slice of the protocol is consistent with observable on-chain behavior after the change.
- The pull request explains verification performed, scope boundaries, and any residual assumptions or risks.

### Constraints for contributors

- Describe **outcomes**, **invariants**, and **evidence**, not a single “right” internal design unless the issue title already names a concrete subsystem.
- Prefer **observable** guarantees: state transitions, balances, authorization failures, emitted events, error classifications, and documentation that integrators rely on.
- If something cannot be tested automatically, capture the gap as **audit notes** with explicit rationale and residual risk.

## Requirements and context

- Must be **secure**, **well-tested**, and **documented**.
- Should be **gas-conscious** (Soroban budget) and **easy to audit**.

## Suggested execution

1. Fork the repo and create a branch:
   ```bash
   git checkout -b feature/get-claimable-at
   ```
2. Implement changes
   - **Contract / modules:** `contracts/stream/src/lib.rs`
   - **Comprehensive tests:** `contracts/stream/src/test.rs` and `contracts/stream/tests/integration_suite.rs`
   - **Documentation:** `docs/` (e.g. `streaming.md`, `security.md`, `events.md`)
   - Include **Rust doc comments** on public items (NatSpec-style clarity for auditors)
   - **Validate security assumptions** (CEI ordering, token trust model, auth paths)
3. **Test and commit**
   - Run `cargo test` in `contracts/stream` (and workspace if applicable)
   - Cover **edge cases** (overflow, TTL, paused/cancelled/completed)
   - Attach **test output** summary + **security notes** in the PR

### Example commit message

```
test(stream): cover get_claimable_at simulations
```

## Guidelines

- Aim for **≥95%** coverage on touched Rust modules (`cargo llvm-cov` or project-standard tooling).
- **Clear documentation** (protocol semantics + operator runbooks where relevant)
- **Timeframe:** 96 hours

++++++

---
type: Feature
title: set_admin: rotation, auth, and AdminUpdated event
labels: contracts, soroban, governance
assignees: ''
---

## Description

### Summary

Within the Fluxora streaming contract, this work tightens externally visible assurances for one focused concern. Treasury operators, recipient-facing applications, and third-party auditors must be able to reason about this area using only on-chain observables and published protocol documentation—without inferring hidden rules from how the implementation happens to be structured. The scenarios captured by this issue title must therefore have crisp success semantics, crisp failure semantics, and no silent drift between stored state, user-visible errors, and emitted events. Economic conservation and payout ordering assumptions must be defensible to a treasury or auditor without reference to internal layout. In practical terms, treat “set_admin: rotation, auth, and AdminUpdated event” as the scope boundary: everything materially related to that caption should be covered by evidence (automated or documented) in this change, and anything intentionally excluded should be called out with rationale and residual risk.

**Issue caption:** set_admin: rotation, auth, and AdminUpdated event

### Domain context

This repository hosts the Fluxora streaming contract on Soroban (Stellar). Streams lock funded assets in contract
storage, release entitlement over time according to schedule parameters (including cliffs), and expose explicit roles:
senders who fund and may adjust certain parameters, recipients who claim vested value, administrators who may intervene
under policy, and—where applicable—permissionless callers for maintenance reads or cleanup. Integrators (wallets,
indexers, treasury tooling) depend on coherent state transitions, emitted signals, and error behavior when inputs or
timing are invalid.

### Work to complete

1. Characterize the **intended protocol semantics** for *set_admin: rotation, auth, and AdminUpdated event* in both success and failure cases, treating the **Summary** above as the authoritative scope statement.
2. Map which roles may participate, what each role must prove, and which operations must be impossible for non-authorized actors.
3. Enumerate edge cases driven by time (start, cliff, end, cancellation freeze), numeric ranges, and stream status combinations; capture expectations with tests or explicit audit exceptions.
4. Ensure externally visible behavior—persistent fields, contract errors, and emitted payloads—does not contradict documentation that integrators rely on.

### Definition of done

- An independent reader can confirm *set_admin: rotation, auth, and AdminUpdated event* matches the stated intent using tests, traces, snapshots (where used), or written review notes.
- Documentation for this slice of the protocol is consistent with observable on-chain behavior after the change.
- The pull request explains verification performed, scope boundaries, and any residual assumptions or risks.

### Constraints for contributors

- Describe **outcomes**, **invariants**, and **evidence**, not a single “right” internal design unless the issue title already names a concrete subsystem.
- Prefer **observable** guarantees: state transitions, balances, authorization failures, emitted events, error classifications, and documentation that integrators rely on.
- If something cannot be tested automatically, capture the gap as **audit notes** with explicit rationale and residual risk.

## Requirements and context

- Must be **secure**, **well-tested**, and **documented**.
- Should be **gas-conscious** (Soroban budget) and **easy to audit**.

## Suggested execution

1. Fork the repo and create a branch:
   ```bash
   git checkout -b feature/set-admin-rotation
   ```
2. Implement changes
   - **Contract / modules:** `contracts/stream/src/lib.rs`
   - **Comprehensive tests:** `contracts/stream/src/test.rs` and `contracts/stream/tests/integration_suite.rs`
   - **Documentation:** `docs/` (e.g. `streaming.md`, `security.md`, `events.md`)
   - Include **Rust doc comments** on public items (NatSpec-style clarity for auditors)
   - **Validate security assumptions** (CEI ordering, token trust model, auth paths)
3. **Test and commit**
   - Run `cargo test` in `contracts/stream` (and workspace if applicable)
   - Cover **edge cases** (overflow, TTL, paused/cancelled/completed)
   - Attach **test output** summary + **security notes** in the PR

### Example commit message

```
test(stream): cover admin rotation
```

## Guidelines

- Aim for **≥95%** coverage on touched Rust modules (`cargo llvm-cov` or project-standard tooling).
- **Clear documentation** (protocol semantics + operator runbooks where relevant)
- **Timeframe:** 96 hours

++++++

---
type: Feature
title: set_contract_paused: admin-only governance toggle
labels: contracts, soroban, governance
assignees: ''
---

## Description

### Summary

Within the Fluxora streaming contract, this work tightens externally visible assurances for one focused concern. Treasury operators, recipient-facing applications, and third-party auditors must be able to reason about this area using only on-chain observables and published protocol documentation—without inferring hidden rules from how the implementation happens to be structured. The scenarios captured by this issue title must therefore have crisp success semantics, crisp failure semantics, and no silent drift between stored state, user-visible errors, and emitted events. Economic conservation and payout ordering assumptions must be defensible to a treasury or auditor without reference to internal layout. In practical terms, treat “set_contract_paused: admin-only governance toggle” as the scope boundary: everything materially related to that caption should be covered by evidence (automated or documented) in this change, and anything intentionally excluded should be called out with rationale and residual risk.

**Issue caption:** set_contract_paused: admin-only governance toggle

### Domain context

This repository hosts the Fluxora streaming contract on Soroban (Stellar). Streams lock funded assets in contract
storage, release entitlement over time according to schedule parameters (including cliffs), and expose explicit roles:
senders who fund and may adjust certain parameters, recipients who claim vested value, administrators who may intervene
under policy, and—where applicable—permissionless callers for maintenance reads or cleanup. Integrators (wallets,
indexers, treasury tooling) depend on coherent state transitions, emitted signals, and error behavior when inputs or
timing are invalid.

### Work to complete

1. Characterize the **intended protocol semantics** for *set_contract_paused: admin-only governance toggle* in both success and failure cases, treating the **Summary** above as the authoritative scope statement.
2. Map which roles may participate, what each role must prove, and which operations must be impossible for non-authorized actors.
3. Enumerate edge cases driven by time (start, cliff, end, cancellation freeze), numeric ranges, and stream status combinations; capture expectations with tests or explicit audit exceptions.
4. Ensure externally visible behavior—persistent fields, contract errors, and emitted payloads—does not contradict documentation that integrators rely on.

### Definition of done

- An independent reader can confirm *set_contract_paused: admin-only governance toggle* matches the stated intent using tests, traces, snapshots (where used), or written review notes.
- Documentation for this slice of the protocol is consistent with observable on-chain behavior after the change.
- The pull request explains verification performed, scope boundaries, and any residual assumptions or risks.

### Constraints for contributors

- Describe **outcomes**, **invariants**, and **evidence**, not a single “right” internal design unless the issue title already names a concrete subsystem.
- Prefer **observable** guarantees: state transitions, balances, authorization failures, emitted events, error classifications, and documentation that integrators rely on.
- If something cannot be tested automatically, capture the gap as **audit notes** with explicit rationale and residual risk.

## Requirements and context

- Must be **secure**, **well-tested**, and **documented**.
- Should be **gas-conscious** (Soroban budget) and **easy to audit**.

## Suggested execution

1. Fork the repo and create a branch:
   ```bash
   git checkout -b feature/set-contract-paused
   ```
2. Implement changes
   - **Contract / modules:** `contracts/stream/src/lib.rs`
   - **Comprehensive tests:** `contracts/stream/src/test.rs` and `contracts/stream/tests/integration_suite.rs`
   - **Documentation:** `docs/` (e.g. `streaming.md`, `security.md`, `events.md`)
   - Include **Rust doc comments** on public items (NatSpec-style clarity for auditors)
   - **Validate security assumptions** (CEI ordering, token trust model, auth paths)
3. **Test and commit**
   - Run `cargo test` in `contracts/stream` (and workspace if applicable)
   - Cover **edge cases** (overflow, TTL, paused/cancelled/completed)
   - Attach **test output** summary + **security notes** in the PR

### Example commit message

```
test(stream): cover global pause toggle
```

## Guidelines

- Aim for **≥95%** coverage on touched Rust modules (`cargo llvm-cov` or project-standard tooling).
- **Clear documentation** (protocol semantics + operator runbooks where relevant)
- **Timeframe:** 96 hours

++++++

---
type: Feature
title: update_rate_per_second: monotonic rate increases only
labels: contracts, soroban, schedule
assignees: ''
---

## Description

### Summary

Within the Fluxora streaming contract, this work tightens externally visible assurances for one focused concern. Treasury operators, recipient-facing applications, and third-party auditors must be able to reason about this area using only on-chain observables and published protocol documentation—without inferring hidden rules from how the implementation happens to be structured. The scenarios captured by this issue title must therefore have crisp success semantics, crisp failure semantics, and no silent drift between stored state, user-visible errors, and emitted events. Permissionless maintenance or read paths, if involved, must have clearly documented trust impact for operators. In practical terms, treat “update_rate_per_second: monotonic rate increases only” as the scope boundary: everything materially related to that caption should be covered by evidence (automated or documented) in this change, and anything intentionally excluded should be called out with rationale and residual risk.

**Issue caption:** update_rate_per_second: monotonic rate increases only

### Domain context

This repository hosts the Fluxora streaming contract on Soroban (Stellar). Streams lock funded assets in contract
storage, release entitlement over time according to schedule parameters (including cliffs), and expose explicit roles:
senders who fund and may adjust certain parameters, recipients who claim vested value, administrators who may intervene
under policy, and—where applicable—permissionless callers for maintenance reads or cleanup. Integrators (wallets,
indexers, treasury tooling) depend on coherent state transitions, emitted signals, and error behavior when inputs or
timing are invalid.

### Work to complete

1. Characterize the **intended protocol semantics** for *update_rate_per_second: monotonic rate increases only* in both success and failure cases, treating the **Summary** above as the authoritative scope statement.
2. Map which roles may participate, what each role must prove, and which operations must be impossible for non-authorized actors.
3. Enumerate edge cases driven by time (start, cliff, end, cancellation freeze), numeric ranges, and stream status combinations; capture expectations with tests or explicit audit exceptions.
4. Ensure externally visible behavior—persistent fields, contract errors, and emitted payloads—does not contradict documentation that integrators rely on.

### Definition of done

- An independent reader can confirm *update_rate_per_second: monotonic rate increases only* matches the stated intent using tests, traces, snapshots (where used), or written review notes.
- Documentation for this slice of the protocol is consistent with observable on-chain behavior after the change.
- The pull request explains verification performed, scope boundaries, and any residual assumptions or risks.

### Constraints for contributors

- Describe **outcomes**, **invariants**, and **evidence**, not a single “right” internal design unless the issue title already names a concrete subsystem.
- Prefer **observable** guarantees: state transitions, balances, authorization failures, emitted events, error classifications, and documentation that integrators rely on.
- If something cannot be tested automatically, capture the gap as **audit notes** with explicit rationale and residual risk.

## Requirements and context

- Must be **secure**, **well-tested**, and **documented**.
- Should be **gas-conscious** (Soroban budget) and **easy to audit**.

## Suggested execution

1. Fork the repo and create a branch:
   ```bash
   git checkout -b feature/update-rate
   ```
2. Implement changes
   - **Contract / modules:** `contracts/stream/src/lib.rs`
   - **Comprehensive tests:** `contracts/stream/src/test.rs` and `contracts/stream/tests/integration_suite.rs`
   - **Documentation:** `docs/` (e.g. `streaming.md`, `security.md`, `events.md`)
   - Include **Rust doc comments** on public items (NatSpec-style clarity for auditors)
   - **Validate security assumptions** (CEI ordering, token trust model, auth paths)
3. **Test and commit**
   - Run `cargo test` in `contracts/stream` (and workspace if applicable)
   - Cover **edge cases** (overflow, TTL, paused/cancelled/completed)
   - Attach **test output** summary + **security notes** in the PR

### Example commit message

```
test(stream): cover forward-only rate updates
```

## Guidelines

- Aim for **≥95%** coverage on touched Rust modules (`cargo llvm-cov` or project-standard tooling).
- **Clear documentation** (protocol semantics + operator runbooks where relevant)
- **Timeframe:** 96 hours

++++++

---
type: Task
title: shorten_stream_end_time: refund correctness and invariants
labels: contracts, soroban, schedule, tokens
assignees: ''
---

## Description

### Summary

Within the Fluxora streaming contract, this work tightens externally visible assurances for one focused concern. Treasury operators, recipient-facing applications, and third-party auditors must be able to reason about this area using only on-chain observables and published protocol documentation—without inferring hidden rules from how the implementation happens to be structured. The scenarios captured by this issue title must therefore have crisp success semantics, crisp failure semantics, and no silent drift between stored state, user-visible errors, and emitted events. Time boundaries (start, cliff, end, cancellation freeze) and numeric ranges must be enumerated so entitlement cannot be misread under edge timing. In practical terms, treat “shorten_stream_end_time: refund correctness and invariants” as the scope boundary: everything materially related to that caption should be covered by evidence (automated or documented) in this change, and anything intentionally excluded should be called out with rationale and residual risk.

**Issue caption:** shorten_stream_end_time: refund correctness and invariants

### Domain context

This repository hosts the Fluxora streaming contract on Soroban (Stellar). Streams lock funded assets in contract
storage, release entitlement over time according to schedule parameters (including cliffs), and expose explicit roles:
senders who fund and may adjust certain parameters, recipients who claim vested value, administrators who may intervene
under policy, and—where applicable—permissionless callers for maintenance reads or cleanup. Integrators (wallets,
indexers, treasury tooling) depend on coherent state transitions, emitted signals, and error behavior when inputs or
timing are invalid.

### Work to complete

1. Characterize the **intended protocol semantics** for *shorten_stream_end_time: refund correctness and invariants* in both success and failure cases, treating the **Summary** above as the authoritative scope statement.
2. Map which roles may participate, what each role must prove, and which operations must be impossible for non-authorized actors.
3. Enumerate edge cases driven by time (start, cliff, end, cancellation freeze), numeric ranges, and stream status combinations; capture expectations with tests or explicit audit exceptions.
4. Ensure externally visible behavior—persistent fields, contract errors, and emitted payloads—does not contradict documentation that integrators rely on.

### Definition of done

- An independent reader can confirm *shorten_stream_end_time: refund correctness and invariants* matches the stated intent using tests, traces, snapshots (where used), or written review notes.
- Documentation for this slice of the protocol is consistent with observable on-chain behavior after the change.
- The pull request explains verification performed, scope boundaries, and any residual assumptions or risks.

### Constraints for contributors

- Describe **outcomes**, **invariants**, and **evidence**, not a single “right” internal design unless the issue title already names a concrete subsystem.
- Prefer **observable** guarantees: state transitions, balances, authorization failures, emitted events, error classifications, and documentation that integrators rely on.
- If something cannot be tested automatically, capture the gap as **audit notes** with explicit rationale and residual risk.

## Requirements and context

- Must be **secure**, **well-tested**, and **documented**.
- Should be **gas-conscious** (Soroban budget) and **easy to audit**.

## Suggested execution

1. Fork the repo and create a branch:
   ```bash
   git checkout -b feature/shorten-end
   ```
2. Implement changes
   - **Contract / modules:** `contracts/stream/src/lib.rs`
   - **Comprehensive tests:** `contracts/stream/src/test.rs` and `contracts/stream/tests/integration_suite.rs`
   - **Documentation:** `docs/` (e.g. `streaming.md`, `security.md`, `events.md`)
   - Include **Rust doc comments** on public items (NatSpec-style clarity for auditors)
   - **Validate security assumptions** (CEI ordering, token trust model, auth paths)
3. **Test and commit**
   - Run `cargo test` in `contracts/stream` (and workspace if applicable)
   - Cover **edge cases** (overflow, TTL, paused/cancelled/completed)
   - Attach **test output** summary + **security notes** in the PR

### Example commit message

```
test(stream): cover shorten_stream_end_time
```

## Guidelines

- Aim for **≥95%** coverage on touched Rust modules (`cargo llvm-cov` or project-standard tooling).
- **Clear documentation** (protocol semantics + operator runbooks where relevant)
- **Timeframe:** 96 hours

++++++

---
type: Feature
title: extend_stream_end_time: deposit sufficiency under longer duration
labels: contracts, soroban, schedule
assignees: ''
---

## Description

### Summary

Within the Fluxora streaming contract, this work tightens externally visible assurances for one focused concern. Treasury operators, recipient-facing applications, and third-party auditors must be able to reason about this area using only on-chain observables and published protocol documentation—without inferring hidden rules from how the implementation happens to be structured. The scenarios captured by this issue title must therefore have crisp success semantics, crisp failure semantics, and no silent drift between stored state, user-visible errors, and emitted events. Authorization boundaries for this slice must be explicit: who may trigger each path, and what proof they must supply. In practical terms, treat “extend_stream_end_time: deposit sufficiency under longer duration” as the scope boundary: everything materially related to that caption should be covered by evidence (automated or documented) in this change, and anything intentionally excluded should be called out with rationale and residual risk.

**Issue caption:** extend_stream_end_time: deposit sufficiency under longer duration

### Domain context

This repository hosts the Fluxora streaming contract on Soroban (Stellar). Streams lock funded assets in contract
storage, release entitlement over time according to schedule parameters (including cliffs), and expose explicit roles:
senders who fund and may adjust certain parameters, recipients who claim vested value, administrators who may intervene
under policy, and—where applicable—permissionless callers for maintenance reads or cleanup. Integrators (wallets,
indexers, treasury tooling) depend on coherent state transitions, emitted signals, and error behavior when inputs or
timing are invalid.

### Work to complete

1. Characterize the **intended protocol semantics** for *extend_stream_end_time: deposit sufficiency under longer duration* in both success and failure cases, treating the **Summary** above as the authoritative scope statement.
2. Map which roles may participate, what each role must prove, and which operations must be impossible for non-authorized actors.
3. Enumerate edge cases driven by time (start, cliff, end, cancellation freeze), numeric ranges, and stream status combinations; capture expectations with tests or explicit audit exceptions.
4. Ensure externally visible behavior—persistent fields, contract errors, and emitted payloads—does not contradict documentation that integrators rely on.

### Definition of done

- An independent reader can confirm *extend_stream_end_time: deposit sufficiency under longer duration* matches the stated intent using tests, traces, snapshots (where used), or written review notes.
- Documentation for this slice of the protocol is consistent with observable on-chain behavior after the change.
- The pull request explains verification performed, scope boundaries, and any residual assumptions or risks.

### Constraints for contributors

- Describe **outcomes**, **invariants**, and **evidence**, not a single “right” internal design unless the issue title already names a concrete subsystem.
- Prefer **observable** guarantees: state transitions, balances, authorization failures, emitted events, error classifications, and documentation that integrators rely on.
- If something cannot be tested automatically, capture the gap as **audit notes** with explicit rationale and residual risk.

## Requirements and context

- Must be **secure**, **well-tested**, and **documented**.
- Should be **gas-conscious** (Soroban budget) and **easy to audit**.

## Suggested execution

1. Fork the repo and create a branch:
   ```bash
   git checkout -b feature/extend-end
   ```
2. Implement changes
   - **Contract / modules:** `contracts/stream/src/lib.rs`
   - **Comprehensive tests:** `contracts/stream/src/test.rs` and `contracts/stream/tests/integration_suite.rs`
   - **Documentation:** `docs/` (e.g. `streaming.md`, `security.md`, `events.md`)
   - Include **Rust doc comments** on public items (NatSpec-style clarity for auditors)
   - **Validate security assumptions** (CEI ordering, token trust model, auth paths)
3. **Test and commit**
   - Run `cargo test` in `contracts/stream` (and workspace if applicable)
   - Cover **edge cases** (overflow, TTL, paused/cancelled/completed)
   - Attach **test output** summary + **security notes** in the PR

### Example commit message

```
test(stream): cover extend_stream_end_time
```

## Guidelines

- Aim for **≥95%** coverage on touched Rust modules (`cargo llvm-cov` or project-standard tooling).
- **Clear documentation** (protocol semantics + operator runbooks where relevant)
- **Timeframe:** 96 hours

++++++

---
type: Feature
title: top_up_stream: pull + deposit increase + event
labels: contracts, soroban, treasury, tokens
assignees: ''
---

## Description

### Summary

Within the Fluxora streaming contract, this work tightens externally visible assurances for one focused concern. Treasury operators, recipient-facing applications, and third-party auditors must be able to reason about this area using only on-chain observables and published protocol documentation—without inferring hidden rules from how the implementation happens to be structured. The scenarios captured by this issue title must therefore have crisp success semantics, crisp failure semantics, and no silent drift between stored state, user-visible errors, and emitted events. Economic conservation and payout ordering assumptions must be defensible to a treasury or auditor without reference to internal layout. In practical terms, treat “top_up_stream: pull + deposit increase + event” as the scope boundary: everything materially related to that caption should be covered by evidence (automated or documented) in this change, and anything intentionally excluded should be called out with rationale and residual risk.

**Issue caption:** top_up_stream: pull + deposit increase + event

### Domain context

This repository hosts the Fluxora streaming contract on Soroban (Stellar). Streams lock funded assets in contract
storage, release entitlement over time according to schedule parameters (including cliffs), and expose explicit roles:
senders who fund and may adjust certain parameters, recipients who claim vested value, administrators who may intervene
under policy, and—where applicable—permissionless callers for maintenance reads or cleanup. Integrators (wallets,
indexers, treasury tooling) depend on coherent state transitions, emitted signals, and error behavior when inputs or
timing are invalid.

### Work to complete

1. Characterize the **intended protocol semantics** for *top_up_stream: pull + deposit increase + event* in both success and failure cases, treating the **Summary** above as the authoritative scope statement.
2. Map which roles may participate, what each role must prove, and which operations must be impossible for non-authorized actors.
3. Enumerate edge cases driven by time (start, cliff, end, cancellation freeze), numeric ranges, and stream status combinations; capture expectations with tests or explicit audit exceptions.
4. Ensure externally visible behavior—persistent fields, contract errors, and emitted payloads—does not contradict documentation that integrators rely on.

### Definition of done

- An independent reader can confirm *top_up_stream: pull + deposit increase + event* matches the stated intent using tests, traces, snapshots (where used), or written review notes.
- Documentation for this slice of the protocol is consistent with observable on-chain behavior after the change.
- The pull request explains verification performed, scope boundaries, and any residual assumptions or risks.

### Constraints for contributors

- Describe **outcomes**, **invariants**, and **evidence**, not a single “right” internal design unless the issue title already names a concrete subsystem.
- Prefer **observable** guarantees: state transitions, balances, authorization failures, emitted events, error classifications, and documentation that integrators rely on.
- If something cannot be tested automatically, capture the gap as **audit notes** with explicit rationale and residual risk.

## Requirements and context

- Must be **secure**, **well-tested**, and **documented**.
- Should be **gas-conscious** (Soroban budget) and **easy to audit**.

## Suggested execution

1. Fork the repo and create a branch:
   ```bash
   git checkout -b feature/top-up-stream
   ```
2. Implement changes
   - **Contract / modules:** `contracts/stream/src/lib.rs`
   - **Comprehensive tests:** `contracts/stream/src/test.rs` and `contracts/stream/tests/integration_suite.rs`
   - **Documentation:** `docs/` (e.g. `streaming.md`, `security.md`, `events.md`)
   - Include **Rust doc comments** on public items (NatSpec-style clarity for auditors)
   - **Validate security assumptions** (CEI ordering, token trust model, auth paths)
3. **Test and commit**
   - Run `cargo test` in `contracts/stream` (and workspace if applicable)
   - Cover **edge cases** (overflow, TTL, paused/cancelled/completed)
   - Attach **test output** summary + **security notes** in the PR

### Example commit message

```
test(stream): cover top_up_stream
```

## Guidelines

- Aim for **≥95%** coverage on touched Rust modules (`cargo llvm-cov` or project-standard tooling).
- **Clear documentation** (protocol semantics + operator runbooks where relevant)
- **Timeframe:** 96 hours

++++++

---
type: Feature
title: close_completed_stream: permissionless cleanup and events
labels: contracts, soroban, storage
assignees: ''
---

## Description

### Summary

Within the Fluxora streaming contract, this work tightens externally visible assurances for one focused concern. Treasury operators, recipient-facing applications, and third-party auditors must be able to reason about this area using only on-chain observables and published protocol documentation—without inferring hidden rules from how the implementation happens to be structured. The scenarios captured by this issue title must therefore have crisp success semantics, crisp failure semantics, and no silent drift between stored state, user-visible errors, and emitted events. Time boundaries (start, cliff, end, cancellation freeze) and numeric ranges must be enumerated so entitlement cannot be misread under edge timing. In practical terms, treat “close_completed_stream: permissionless cleanup and events” as the scope boundary: everything materially related to that caption should be covered by evidence (automated or documented) in this change, and anything intentionally excluded should be called out with rationale and residual risk.

**Issue caption:** close_completed_stream: permissionless cleanup and events

### Domain context

This repository hosts the Fluxora streaming contract on Soroban (Stellar). Streams lock funded assets in contract
storage, release entitlement over time according to schedule parameters (including cliffs), and expose explicit roles:
senders who fund and may adjust certain parameters, recipients who claim vested value, administrators who may intervene
under policy, and—where applicable—permissionless callers for maintenance reads or cleanup. Integrators (wallets,
indexers, treasury tooling) depend on coherent state transitions, emitted signals, and error behavior when inputs or
timing are invalid.

### Work to complete

1. Characterize the **intended protocol semantics** for *close_completed_stream: permissionless cleanup and events* in both success and failure cases, treating the **Summary** above as the authoritative scope statement.
2. Map which roles may participate, what each role must prove, and which operations must be impossible for non-authorized actors.
3. Enumerate edge cases driven by time (start, cliff, end, cancellation freeze), numeric ranges, and stream status combinations; capture expectations with tests or explicit audit exceptions.
4. Ensure externally visible behavior—persistent fields, contract errors, and emitted payloads—does not contradict documentation that integrators rely on.

### Definition of done

- An independent reader can confirm *close_completed_stream: permissionless cleanup and events* matches the stated intent using tests, traces, snapshots (where used), or written review notes.
- Documentation for this slice of the protocol is consistent with observable on-chain behavior after the change.
- The pull request explains verification performed, scope boundaries, and any residual assumptions or risks.

### Constraints for contributors

- Describe **outcomes**, **invariants**, and **evidence**, not a single “right” internal design unless the issue title already names a concrete subsystem.
- Prefer **observable** guarantees: state transitions, balances, authorization failures, emitted events, error classifications, and documentation that integrators rely on.
- If something cannot be tested automatically, capture the gap as **audit notes** with explicit rationale and residual risk.

## Requirements and context

- Must be **secure**, **well-tested**, and **documented**.
- Should be **gas-conscious** (Soroban budget) and **easy to audit**.

## Suggested execution

1. Fork the repo and create a branch:
   ```bash
   git checkout -b feature/close-completed
   ```
2. Implement changes
   - **Contract / modules:** `contracts/stream/src/lib.rs`
   - **Comprehensive tests:** `contracts/stream/src/test.rs` and `contracts/stream/tests/integration_suite.rs`
   - **Documentation:** `docs/` (e.g. `streaming.md`, `security.md`, `events.md`)
   - Include **Rust doc comments** on public items (NatSpec-style clarity for auditors)
   - **Validate security assumptions** (CEI ordering, token trust model, auth paths)
3. **Test and commit**
   - Run `cargo test` in `contracts/stream` (and workspace if applicable)
   - Cover **edge cases** (overflow, TTL, paused/cancelled/completed)
   - Attach **test output** summary + **security notes** in the PR

### Example commit message

```
test(stream): cover close_completed_stream cleanup
```

## Guidelines

- Aim for **≥95%** coverage on touched Rust modules (`cargo llvm-cov` or project-standard tooling).
- **Clear documentation** (protocol semantics + operator runbooks where relevant)
- **Timeframe:** 96 hours

++++++

---
type: Feature
title: version(): CONTRACT_VERSION discovery for integrators
labels: contracts, soroban, tooling
assignees: ''
---

## Description

### Summary

Within the Fluxora streaming contract, this work tightens externally visible assurances for one focused concern. Treasury operators, recipient-facing applications, and third-party auditors must be able to reason about this area using only on-chain observables and published protocol documentation—without inferring hidden rules from how the implementation happens to be structured. The scenarios captured by this issue title must therefore have crisp success semantics, crisp failure semantics, and no silent drift between stored state, user-visible errors, and emitted events. Authorization boundaries for this slice must be explicit: who may trigger each path, and what proof they must supply. In practical terms, treat “version(): CONTRACT_VERSION discovery for integrators” as the scope boundary: everything materially related to that caption should be covered by evidence (automated or documented) in this change, and anything intentionally excluded should be called out with rationale and residual risk.

**Issue caption:** version(): CONTRACT_VERSION discovery for integrators

### Domain context

This repository hosts the Fluxora streaming contract on Soroban (Stellar). Streams lock funded assets in contract
storage, release entitlement over time according to schedule parameters (including cliffs), and expose explicit roles:
senders who fund and may adjust certain parameters, recipients who claim vested value, administrators who may intervene
under policy, and—where applicable—permissionless callers for maintenance reads or cleanup. Integrators (wallets,
indexers, treasury tooling) depend on coherent state transitions, emitted signals, and error behavior when inputs or
timing are invalid.

### Work to complete

1. Characterize the **intended protocol semantics** for *version(): CONTRACT_VERSION discovery for integrators* in both success and failure cases, treating the **Summary** above as the authoritative scope statement.
2. Map which roles may participate, what each role must prove, and which operations must be impossible for non-authorized actors.
3. Enumerate edge cases driven by time (start, cliff, end, cancellation freeze), numeric ranges, and stream status combinations; capture expectations with tests or explicit audit exceptions.
4. Ensure externally visible behavior—persistent fields, contract errors, and emitted payloads—does not contradict documentation that integrators rely on.

### Definition of done

- An independent reader can confirm *version(): CONTRACT_VERSION discovery for integrators* matches the stated intent using tests, traces, snapshots (where used), or written review notes.
- Documentation for this slice of the protocol is consistent with observable on-chain behavior after the change.
- The pull request explains verification performed, scope boundaries, and any residual assumptions or risks.

### Constraints for contributors

- Describe **outcomes**, **invariants**, and **evidence**, not a single “right” internal design unless the issue title already names a concrete subsystem.
- Prefer **observable** guarantees: state transitions, balances, authorization failures, emitted events, error classifications, and documentation that integrators rely on.
- If something cannot be tested automatically, capture the gap as **audit notes** with explicit rationale and residual risk.

## Requirements and context

- Must be **secure**, **well-tested**, and **documented**.
- Should be **gas-conscious** (Soroban budget) and **easy to audit**.

## Suggested execution

1. Fork the repo and create a branch:
   ```bash
   git checkout -b feature/contract-version-doc
   ```
2. Implement changes
   - **Contract / modules:** `contracts/stream/src/lib.rs`
   - **Comprehensive tests:** `contracts/stream/src/test.rs` and `contracts/stream/tests/integration_suite.rs`
   - **Documentation:** `docs/` (e.g. `streaming.md`, `security.md`, `events.md`)
   - Include **Rust doc comments** on public items (NatSpec-style clarity for auditors)
   - **Validate security assumptions** (CEI ordering, token trust model, auth paths)
3. **Test and commit**
   - Run `cargo test` in `contracts/stream` (and workspace if applicable)
   - Cover **edge cases** (overflow, TTL, paused/cancelled/completed)
   - Attach **test output** summary + **security notes** in the PR

### Example commit message

```
docs(stream): document version() usage
```

## Guidelines

- Aim for **≥95%** coverage on touched Rust modules (`cargo llvm-cov` or project-standard tooling).
- **Clear documentation** (protocol semantics + operator runbooks where relevant)
- **Timeframe:** 24 hours

++++++

---
type: Task
title: Token helpers audit: pull_token / push_token centralization
labels: contracts, soroban, security, tokens
assignees: ''
---

## Description

### Summary

Within the Fluxora streaming contract, this work tightens externally visible assurances for one focused concern. Treasury operators, recipient-facing applications, and third-party auditors must be able to reason about this area using only on-chain observables and published protocol documentation—without inferring hidden rules from how the implementation happens to be structured. The scenarios captured by this issue title must therefore have crisp success semantics, crisp failure semantics, and no silent drift between stored state, user-visible errors, and emitted events. Permissionless maintenance or read paths, if involved, must have clearly documented trust impact for operators. In practical terms, treat “Token helpers audit: pull_token / push_token centralization” as the scope boundary: everything materially related to that caption should be covered by evidence (automated or documented) in this change, and anything intentionally excluded should be called out with rationale and residual risk.

**Issue caption:** Token helpers audit: pull_token / push_token centralization

### Domain context

This repository hosts the Fluxora streaming contract on Soroban (Stellar). Streams lock funded assets in contract
storage, release entitlement over time according to schedule parameters (including cliffs), and expose explicit roles:
senders who fund and may adjust certain parameters, recipients who claim vested value, administrators who may intervene
under policy, and—where applicable—permissionless callers for maintenance reads or cleanup. Integrators (wallets,
indexers, treasury tooling) depend on coherent state transitions, emitted signals, and error behavior when inputs or
timing are invalid.

### Work to complete

1. Characterize the **intended protocol semantics** for *Token helpers audit: pull_token / push_token centralization* in both success and failure cases, treating the **Summary** above as the authoritative scope statement.
2. Map which roles may participate, what each role must prove, and which operations must be impossible for non-authorized actors.
3. Enumerate edge cases driven by time (start, cliff, end, cancellation freeze), numeric ranges, and stream status combinations; capture expectations with tests or explicit audit exceptions.
4. Ensure externally visible behavior—persistent fields, contract errors, and emitted payloads—does not contradict documentation that integrators rely on.

### Definition of done

- An independent reader can confirm *Token helpers audit: pull_token / push_token centralization* matches the stated intent using tests, traces, snapshots (where used), or written review notes.
- Documentation for this slice of the protocol is consistent with observable on-chain behavior after the change.
- The pull request explains verification performed, scope boundaries, and any residual assumptions or risks.

### Constraints for contributors

- Describe **outcomes**, **invariants**, and **evidence**, not a single “right” internal design unless the issue title already names a concrete subsystem.
- Prefer **observable** guarantees: state transitions, balances, authorization failures, emitted events, error classifications, and documentation that integrators rely on.
- If something cannot be tested automatically, capture the gap as **audit notes** with explicit rationale and residual risk.

## Requirements and context

- Must be **secure**, **well-tested**, and **documented**.
- Should be **gas-conscious** (Soroban budget) and **easy to audit**.

## Suggested execution

1. Fork the repo and create a branch:
   ```bash
   git checkout -b feature/token-helpers-audit
   ```
2. Implement changes
   - **Contract / modules:** `contracts/stream/src/lib.rs`
   - **Comprehensive tests:** `contracts/stream/src/test.rs` and `contracts/stream/tests/integration_suite.rs`
   - **Documentation:** `docs/` (e.g. `streaming.md`, `security.md`, `events.md`)
   - Include **Rust doc comments** on public items (NatSpec-style clarity for auditors)
   - **Validate security assumptions** (CEI ordering, token trust model, auth paths)
3. **Test and commit**
   - Run `cargo test` in `contracts/stream` (and workspace if applicable)
   - Cover **edge cases** (overflow, TTL, paused/cancelled/completed)
   - Attach **test output** summary + **security notes** in the PR

### Example commit message

```
docs(stream): centralize token transfer review notes
```

## Guidelines

- Aim for **≥95%** coverage on touched Rust modules (`cargo llvm-cov` or project-standard tooling).
- **Clear documentation** (protocol semantics + operator runbooks where relevant)
- **Timeframe:** 96 hours

++++++

---
type: Feature
title: accrual.rs: pure math unit tests (cliff, end cap, overflow)
labels: contracts, soroban, math, testing
assignees: ''
---

## Description

### Summary

Within the Fluxora streaming contract, this work tightens externally visible assurances for one focused concern. Treasury operators, recipient-facing applications, and third-party auditors must be able to reason about this area using only on-chain observables and published protocol documentation—without inferring hidden rules from how the implementation happens to be structured. The scenarios captured by this issue title must therefore have crisp success semantics, crisp failure semantics, and no silent drift between stored state, user-visible errors, and emitted events. Emitted signals and error classifications must remain coherent for indexers and wallets building on this protocol surface. In practical terms, treat “accrual.rs: pure math unit tests (cliff, end cap, overflow)” as the scope boundary: everything materially related to that caption should be covered by evidence (automated or documented) in this change, and anything intentionally excluded should be called out with rationale and residual risk.

**Issue caption:** accrual.rs: pure math unit tests (cliff, end cap, overflow)

### Domain context

This repository hosts the Fluxora streaming contract on Soroban (Stellar). Streams lock funded assets in contract
storage, release entitlement over time according to schedule parameters (including cliffs), and expose explicit roles:
senders who fund and may adjust certain parameters, recipients who claim vested value, administrators who may intervene
under policy, and—where applicable—permissionless callers for maintenance reads or cleanup. Integrators (wallets,
indexers, treasury tooling) depend on coherent state transitions, emitted signals, and error behavior when inputs or
timing are invalid.

### Work to complete

1. Characterize the **intended protocol semantics** for *accrual.rs: pure math unit tests (cliff, end cap, overflow)* in both success and failure cases, treating the **Summary** above as the authoritative scope statement.
2. Map which roles may participate, what each role must prove, and which operations must be impossible for non-authorized actors.
3. Enumerate edge cases driven by time (start, cliff, end, cancellation freeze), numeric ranges, and stream status combinations; capture expectations with tests or explicit audit exceptions.
4. Ensure externally visible behavior—persistent fields, contract errors, and emitted payloads—does not contradict documentation that integrators rely on.

### Definition of done

- An independent reader can confirm *accrual.rs: pure math unit tests (cliff, end cap, overflow)* matches the stated intent using tests, traces, snapshots (where used), or written review notes.
- Documentation for this slice of the protocol is consistent with observable on-chain behavior after the change.
- The pull request explains verification performed, scope boundaries, and any residual assumptions or risks.

### Constraints for contributors

- Describe **outcomes**, **invariants**, and **evidence**, not a single “right” internal design unless the issue title already names a concrete subsystem.
- Prefer **observable** guarantees: state transitions, balances, authorization failures, emitted events, error classifications, and documentation that integrators rely on.
- If something cannot be tested automatically, capture the gap as **audit notes** with explicit rationale and residual risk.

## Requirements and context

- Must be **secure**, **well-tested**, and **documented**.
- Should be **gas-conscious** (Soroban budget) and **easy to audit**.

## Suggested execution

1. Fork the repo and create a branch:
   ```bash
   git checkout -b feature/accrual-unit-tests
   ```
2. Implement changes
   - **Contract / modules:** `contracts/stream/src/lib.rs`
   - **Comprehensive tests:** `contracts/stream/src/test.rs` and `contracts/stream/tests/integration_suite.rs`
   - **Documentation:** `docs/` (e.g. `streaming.md`, `security.md`, `events.md`)
   - Include **Rust doc comments** on public items (NatSpec-style clarity for auditors)
   - **Validate security assumptions** (CEI ordering, token trust model, auth paths)
3. **Test and commit**
   - Run `cargo test` in `contracts/stream` (and workspace if applicable)
   - Cover **edge cases** (overflow, TTL, paused/cancelled/completed)
   - Attach **test output** summary + **security notes** in the PR

### Example commit message

```
test(accrual): expand pure math coverage
```

## Guidelines

- Aim for **≥95%** coverage on touched Rust modules (`cargo llvm-cov` or project-standard tooling).
- **Clear documentation** (protocol semantics + operator runbooks where relevant)
- **Timeframe:** 96 hours

++++++

---
type: Feature
title: accrual property tests: bounds and monotonicity
labels: contracts, soroban, math, testing
assignees: ''
---

## Description

### Summary

Within the Fluxora streaming contract, this work tightens externally visible assurances for one focused concern. Treasury operators, recipient-facing applications, and third-party auditors must be able to reason about this area using only on-chain observables and published protocol documentation—without inferring hidden rules from how the implementation happens to be structured. The scenarios captured by this issue title must therefore have crisp success semantics, crisp failure semantics, and no silent drift between stored state, user-visible errors, and emitted events. Emitted signals and error classifications must remain coherent for indexers and wallets building on this protocol surface. In practical terms, treat “accrual property tests: bounds and monotonicity” as the scope boundary: everything materially related to that caption should be covered by evidence (automated or documented) in this change, and anything intentionally excluded should be called out with rationale and residual risk.

**Issue caption:** accrual property tests: bounds and monotonicity

### Domain context

This repository hosts the Fluxora streaming contract on Soroban (Stellar). Streams lock funded assets in contract
storage, release entitlement over time according to schedule parameters (including cliffs), and expose explicit roles:
senders who fund and may adjust certain parameters, recipients who claim vested value, administrators who may intervene
under policy, and—where applicable—permissionless callers for maintenance reads or cleanup. Integrators (wallets,
indexers, treasury tooling) depend on coherent state transitions, emitted signals, and error behavior when inputs or
timing are invalid.

### Work to complete

1. Characterize the **intended protocol semantics** for *accrual property tests: bounds and monotonicity* in both success and failure cases, treating the **Summary** above as the authoritative scope statement.
2. Map which roles may participate, what each role must prove, and which operations must be impossible for non-authorized actors.
3. Enumerate edge cases driven by time (start, cliff, end, cancellation freeze), numeric ranges, and stream status combinations; capture expectations with tests or explicit audit exceptions.
4. Ensure externally visible behavior—persistent fields, contract errors, and emitted payloads—does not contradict documentation that integrators rely on.

### Definition of done

- An independent reader can confirm *accrual property tests: bounds and monotonicity* matches the stated intent using tests, traces, snapshots (where used), or written review notes.
- Documentation for this slice of the protocol is consistent with observable on-chain behavior after the change.
- The pull request explains verification performed, scope boundaries, and any residual assumptions or risks.

### Constraints for contributors

- Describe **outcomes**, **invariants**, and **evidence**, not a single “right” internal design unless the issue title already names a concrete subsystem.
- Prefer **observable** guarantees: state transitions, balances, authorization failures, emitted events, error classifications, and documentation that integrators rely on.
- If something cannot be tested automatically, capture the gap as **audit notes** with explicit rationale and residual risk.

## Requirements and context

- Must be **secure**, **well-tested**, and **documented**.
- Should be **gas-conscious** (Soroban budget) and **easy to audit**.

## Suggested execution

1. Fork the repo and create a branch:
   ```bash
   git checkout -b feature/accrual-properties
   ```
2. Implement changes
   - **Contract / modules:** `contracts/stream/src/lib.rs`
   - **Comprehensive tests:** `contracts/stream/src/test.rs` and `contracts/stream/tests/integration_suite.rs`
   - **Documentation:** `docs/` (e.g. `streaming.md`, `security.md`, `events.md`)
   - Include **Rust doc comments** on public items (NatSpec-style clarity for auditors)
   - **Validate security assumptions** (CEI ordering, token trust model, auth paths)
3. **Test and commit**
   - Run `cargo test` in `contracts/stream` (and workspace if applicable)
   - Cover **edge cases** (overflow, TTL, paused/cancelled/completed)
   - Attach **test output** summary + **security notes** in the PR

### Example commit message

```
test(accrual): add property monotonicity tests
```

## Guidelines

- Aim for **≥95%** coverage on touched Rust modules (`cargo llvm-cov` or project-standard tooling).
- **Clear documentation** (protocol semantics + operator runbooks where relevant)
- **Timeframe:** 96 hours

++++++

---
type: Feature
title: Integration suite: end-to-end stream lifecycle (SAC)
labels: contracts, soroban, integration, testing
assignees: ''
---

## Description

### Summary

Within the Fluxora streaming contract, this work tightens externally visible assurances for one focused concern. Treasury operators, recipient-facing applications, and third-party auditors must be able to reason about this area using only on-chain observables and published protocol documentation—without inferring hidden rules from how the implementation happens to be structured. The scenarios captured by this issue title must therefore have crisp success semantics, crisp failure semantics, and no silent drift between stored state, user-visible errors, and emitted events. Authorization boundaries for this slice must be explicit: who may trigger each path, and what proof they must supply. In practical terms, treat “Integration suite: end-to-end stream lifecycle (SAC)” as the scope boundary: everything materially related to that caption should be covered by evidence (automated or documented) in this change, and anything intentionally excluded should be called out with rationale and residual risk.

**Issue caption:** Integration suite: end-to-end stream lifecycle (SAC)

### Domain context

This repository hosts the Fluxora streaming contract on Soroban (Stellar). Streams lock funded assets in contract
storage, release entitlement over time according to schedule parameters (including cliffs), and expose explicit roles:
senders who fund and may adjust certain parameters, recipients who claim vested value, administrators who may intervene
under policy, and—where applicable—permissionless callers for maintenance reads or cleanup. Integrators (wallets,
indexers, treasury tooling) depend on coherent state transitions, emitted signals, and error behavior when inputs or
timing are invalid.

### Work to complete

1. Characterize the **intended protocol semantics** for *Integration suite: end-to-end stream lifecycle (SAC)* in both success and failure cases, treating the **Summary** above as the authoritative scope statement.
2. Map which roles may participate, what each role must prove, and which operations must be impossible for non-authorized actors.
3. Enumerate edge cases driven by time (start, cliff, end, cancellation freeze), numeric ranges, and stream status combinations; capture expectations with tests or explicit audit exceptions.
4. Ensure externally visible behavior—persistent fields, contract errors, and emitted payloads—does not contradict documentation that integrators rely on.

### Definition of done

- An independent reader can confirm *Integration suite: end-to-end stream lifecycle (SAC)* matches the stated intent using tests, traces, snapshots (where used), or written review notes.
- Documentation for this slice of the protocol is consistent with observable on-chain behavior after the change.
- The pull request explains verification performed, scope boundaries, and any residual assumptions or risks.

### Constraints for contributors

- Describe **outcomes**, **invariants**, and **evidence**, not a single “right” internal design unless the issue title already names a concrete subsystem.
- Prefer **observable** guarantees: state transitions, balances, authorization failures, emitted events, error classifications, and documentation that integrators rely on.
- If something cannot be tested automatically, capture the gap as **audit notes** with explicit rationale and residual risk.

## Requirements and context

- Must be **secure**, **well-tested**, and **documented**.
- Should be **gas-conscious** (Soroban budget) and **easy to audit**.

## Suggested execution

1. Fork the repo and create a branch:
   ```bash
   git checkout -b feature/integration-lifecycle
   ```
2. Implement changes
   - **Contract / modules:** `contracts/stream/src/lib.rs`
   - **Comprehensive tests:** `contracts/stream/src/test.rs` and `contracts/stream/tests/integration_suite.rs`
   - **Documentation:** `docs/` (e.g. `streaming.md`, `security.md`, `events.md`)
   - Include **Rust doc comments** on public items (NatSpec-style clarity for auditors)
   - **Validate security assumptions** (CEI ordering, token trust model, auth paths)
3. **Test and commit**
   - Run `cargo test` in `contracts/stream` (and workspace if applicable)
   - Cover **edge cases** (overflow, TTL, paused/cancelled/completed)
   - Attach **test output** summary + **security notes** in the PR

### Example commit message

```
test(integration): e2e stream lifecycle
```

## Guidelines

- Aim for **≥95%** coverage on touched Rust modules (`cargo llvm-cov` or project-standard tooling).
- **Clear documentation** (protocol semantics + operator runbooks where relevant)
- **Timeframe:** 96 hours

++++++

---
type: Feature
title: Integration suite: adversarial auth (strict mock auths)
labels: contracts, soroban, security, testing
assignees: ''
---

## Description

### Summary

Within the Fluxora streaming contract, this work tightens externally visible assurances for one focused concern. Treasury operators, recipient-facing applications, and third-party auditors must be able to reason about this area using only on-chain observables and published protocol documentation—without inferring hidden rules from how the implementation happens to be structured. The scenarios captured by this issue title must therefore have crisp success semantics, crisp failure semantics, and no silent drift between stored state, user-visible errors, and emitted events. Economic conservation and payout ordering assumptions must be defensible to a treasury or auditor without reference to internal layout. In practical terms, treat “Integration suite: adversarial auth (strict mock auths)” as the scope boundary: everything materially related to that caption should be covered by evidence (automated or documented) in this change, and anything intentionally excluded should be called out with rationale and residual risk.

**Issue caption:** Integration suite: adversarial auth (strict mock auths)

### Domain context

This repository hosts the Fluxora streaming contract on Soroban (Stellar). Streams lock funded assets in contract
storage, release entitlement over time according to schedule parameters (including cliffs), and expose explicit roles:
senders who fund and may adjust certain parameters, recipients who claim vested value, administrators who may intervene
under policy, and—where applicable—permissionless callers for maintenance reads or cleanup. Integrators (wallets,
indexers, treasury tooling) depend on coherent state transitions, emitted signals, and error behavior when inputs or
timing are invalid.

### Work to complete

1. Characterize the **intended protocol semantics** for *Integration suite: adversarial auth (strict mock auths)* in both success and failure cases, treating the **Summary** above as the authoritative scope statement.
2. Map which roles may participate, what each role must prove, and which operations must be impossible for non-authorized actors.
3. Enumerate edge cases driven by time (start, cliff, end, cancellation freeze), numeric ranges, and stream status combinations; capture expectations with tests or explicit audit exceptions.
4. Ensure externally visible behavior—persistent fields, contract errors, and emitted payloads—does not contradict documentation that integrators rely on.

### Definition of done

- An independent reader can confirm *Integration suite: adversarial auth (strict mock auths)* matches the stated intent using tests, traces, snapshots (where used), or written review notes.
- Documentation for this slice of the protocol is consistent with observable on-chain behavior after the change.
- The pull request explains verification performed, scope boundaries, and any residual assumptions or risks.

### Constraints for contributors

- Describe **outcomes**, **invariants**, and **evidence**, not a single “right” internal design unless the issue title already names a concrete subsystem.
- Prefer **observable** guarantees: state transitions, balances, authorization failures, emitted events, error classifications, and documentation that integrators rely on.
- If something cannot be tested automatically, capture the gap as **audit notes** with explicit rationale and residual risk.

## Requirements and context

- Must be **secure**, **well-tested**, and **documented**.
- Should be **gas-conscious** (Soroban budget) and **easy to audit**.

## Suggested execution

1. Fork the repo and create a branch:
   ```bash
   git checkout -b feature/integration-auth-negative
   ```
2. Implement changes
   - **Contract / modules:** `contracts/stream/src/lib.rs`
   - **Comprehensive tests:** `contracts/stream/src/test.rs` and `contracts/stream/tests/integration_suite.rs`
   - **Documentation:** `docs/` (e.g. `streaming.md`, `security.md`, `events.md`)
   - Include **Rust doc comments** on public items (NatSpec-style clarity for auditors)
   - **Validate security assumptions** (CEI ordering, token trust model, auth paths)
3. **Test and commit**
   - Run `cargo test` in `contracts/stream` (and workspace if applicable)
   - Cover **edge cases** (overflow, TTL, paused/cancelled/completed)
   - Attach **test output** summary + **security notes** in the PR

### Example commit message

```
test(integration): negative authorization cases
```

## Guidelines

- Aim for **≥95%** coverage on touched Rust modules (`cargo llvm-cov` or project-standard tooling).
- **Clear documentation** (protocol semantics + operator runbooks where relevant)
- **Timeframe:** 96 hours

++++++

---
type: Task
title: Events catalog: align symbol names with docs/events.md
labels: contracts, soroban, docs, indexing
assignees: ''
---

## Description

### Summary

Within the Fluxora streaming contract, this work tightens externally visible assurances for one focused concern. Treasury operators, recipient-facing applications, and third-party auditors must be able to reason about this area using only on-chain observables and published protocol documentation—without inferring hidden rules from how the implementation happens to be structured. The scenarios captured by this issue title must therefore have crisp success semantics, crisp failure semantics, and no silent drift between stored state, user-visible errors, and emitted events. Permissionless maintenance or read paths, if involved, must have clearly documented trust impact for operators. In practical terms, treat “Events catalog: align symbol names with docs/events.md” as the scope boundary: everything materially related to that caption should be covered by evidence (automated or documented) in this change, and anything intentionally excluded should be called out with rationale and residual risk.

**Issue caption:** Events catalog: align symbol names with docs/events.md

### Domain context

This repository hosts the Fluxora streaming contract on Soroban (Stellar). Streams lock funded assets in contract
storage, release entitlement over time according to schedule parameters (including cliffs), and expose explicit roles:
senders who fund and may adjust certain parameters, recipients who claim vested value, administrators who may intervene
under policy, and—where applicable—permissionless callers for maintenance reads or cleanup. Integrators (wallets,
indexers, treasury tooling) depend on coherent state transitions, emitted signals, and error behavior when inputs or
timing are invalid.

### Work to complete

1. Characterize the **intended protocol semantics** for *Events catalog: align symbol names with docs/events.md* in both success and failure cases, treating the **Summary** above as the authoritative scope statement.
2. Map which roles may participate, what each role must prove, and which operations must be impossible for non-authorized actors.
3. Enumerate edge cases driven by time (start, cliff, end, cancellation freeze), numeric ranges, and stream status combinations; capture expectations with tests or explicit audit exceptions.
4. Ensure externally visible behavior—persistent fields, contract errors, and emitted payloads—does not contradict documentation that integrators rely on.

### Definition of done

- An independent reader can confirm *Events catalog: align symbol names with docs/events.md* matches the stated intent using tests, traces, snapshots (where used), or written review notes.
- Documentation for this slice of the protocol is consistent with observable on-chain behavior after the change.
- The pull request explains verification performed, scope boundaries, and any residual assumptions or risks.

### Constraints for contributors

- Describe **outcomes**, **invariants**, and **evidence**, not a single “right” internal design unless the issue title already names a concrete subsystem.
- Prefer **observable** guarantees: state transitions, balances, authorization failures, emitted events, error classifications, and documentation that integrators rely on.
- If something cannot be tested automatically, capture the gap as **audit notes** with explicit rationale and residual risk.

## Requirements and context

- Must be **secure**, **well-tested**, and **documented**.
- Should be **gas-conscious** (Soroban budget) and **easy to audit**.

## Suggested execution

1. Fork the repo and create a branch:
   ```bash
   git checkout -b feature/events-doc-sync
   ```
2. Implement changes
   - **Contract / modules:** `contracts/stream/src/lib.rs`
   - **Comprehensive tests:** `contracts/stream/src/test.rs` and `contracts/stream/tests/integration_suite.rs`
   - **Documentation:** `docs/` (e.g. `streaming.md`, `security.md`, `events.md`)
   - Include **Rust doc comments** on public items (NatSpec-style clarity for auditors)
   - **Validate security assumptions** (CEI ordering, token trust model, auth paths)
3. **Test and commit**
   - Run `cargo test` in `contracts/stream` (and workspace if applicable)
   - Cover **edge cases** (overflow, TTL, paused/cancelled/completed)
   - Attach **test output** summary + **security notes** in the PR

### Example commit message

```
docs(events): sync event catalog with implementation
```

## Guidelines

- Aim for **≥95%** coverage on touched Rust modules (`cargo llvm-cov` or project-standard tooling).
- **Clear documentation** (protocol semantics + operator runbooks where relevant)
- **Timeframe:** 96 hours

++++++

---
type: Feature
title: ContractError: user-facing mapping for clients
labels: contracts, soroban, docs
assignees: ''
---

## Description

### Summary

Within the Fluxora streaming contract, this work tightens externally visible assurances for one focused concern. Treasury operators, recipient-facing applications, and third-party auditors must be able to reason about this area using only on-chain observables and published protocol documentation—without inferring hidden rules from how the implementation happens to be structured. The scenarios captured by this issue title must therefore have crisp success semantics, crisp failure semantics, and no silent drift between stored state, user-visible errors, and emitted events. Permissionless maintenance or read paths, if involved, must have clearly documented trust impact for operators. In practical terms, treat “ContractError: user-facing mapping for clients” as the scope boundary: everything materially related to that caption should be covered by evidence (automated or documented) in this change, and anything intentionally excluded should be called out with rationale and residual risk.

**Issue caption:** ContractError: user-facing mapping for clients

### Domain context

This repository hosts the Fluxora streaming contract on Soroban (Stellar). Streams lock funded assets in contract
storage, release entitlement over time according to schedule parameters (including cliffs), and expose explicit roles:
senders who fund and may adjust certain parameters, recipients who claim vested value, administrators who may intervene
under policy, and—where applicable—permissionless callers for maintenance reads or cleanup. Integrators (wallets,
indexers, treasury tooling) depend on coherent state transitions, emitted signals, and error behavior when inputs or
timing are invalid.

### Work to complete

1. Characterize the **intended protocol semantics** for *ContractError: user-facing mapping for clients* in both success and failure cases, treating the **Summary** above as the authoritative scope statement.
2. Map which roles may participate, what each role must prove, and which operations must be impossible for non-authorized actors.
3. Enumerate edge cases driven by time (start, cliff, end, cancellation freeze), numeric ranges, and stream status combinations; capture expectations with tests or explicit audit exceptions.
4. Ensure externally visible behavior—persistent fields, contract errors, and emitted payloads—does not contradict documentation that integrators rely on.

### Definition of done

- An independent reader can confirm *ContractError: user-facing mapping for clients* matches the stated intent using tests, traces, snapshots (where used), or written review notes.
- Documentation for this slice of the protocol is consistent with observable on-chain behavior after the change.
- The pull request explains verification performed, scope boundaries, and any residual assumptions or risks.

### Constraints for contributors

- Describe **outcomes**, **invariants**, and **evidence**, not a single “right” internal design unless the issue title already names a concrete subsystem.
- Prefer **observable** guarantees: state transitions, balances, authorization failures, emitted events, error classifications, and documentation that integrators rely on.
- If something cannot be tested automatically, capture the gap as **audit notes** with explicit rationale and residual risk.

## Requirements and context

- Must be **secure**, **well-tested**, and **documented**.
- Should be **gas-conscious** (Soroban budget) and **easy to audit**.

## Suggested execution

1. Fork the repo and create a branch:
   ```bash
   git checkout -b feature/error-doc-contract
   ```
2. Implement changes
   - **Contract / modules:** `contracts/stream/src/lib.rs`
   - **Comprehensive tests:** `contracts/stream/src/test.rs` and `contracts/stream/tests/integration_suite.rs`
   - **Documentation:** `docs/` (e.g. `streaming.md`, `security.md`, `events.md`)
   - Include **Rust doc comments** on public items (NatSpec-style clarity for auditors)
   - **Validate security assumptions** (CEI ordering, token trust model, auth paths)
3. **Test and commit**
   - Run `cargo test` in `contracts/stream` (and workspace if applicable)
   - Cover **edge cases** (overflow, TTL, paused/cancelled/completed)
   - Attach **test output** summary + **security notes** in the PR

### Example commit message

```
docs(errors): map ContractError to scenarios
```

## Guidelines

- Aim for **≥95%** coverage on touched Rust modules (`cargo llvm-cov` or project-standard tooling).
- **Clear documentation** (protocol semantics + operator runbooks where relevant)
- **Timeframe:** 96 hours

++++++

---
type: Feature
title: Snapshot tests: update workflow and CI guidance
labels: contracts, soroban, tooling, testing
assignees: ''
---

## Description

### Summary

Within the Fluxora streaming contract, this work tightens externally visible assurances for one focused concern. Treasury operators, recipient-facing applications, and third-party auditors must be able to reason about this area using only on-chain observables and published protocol documentation—without inferring hidden rules from how the implementation happens to be structured. The scenarios captured by this issue title must therefore have crisp success semantics, crisp failure semantics, and no silent drift between stored state, user-visible errors, and emitted events. Authorization boundaries for this slice must be explicit: who may trigger each path, and what proof they must supply. In practical terms, treat “Snapshot tests: update workflow and CI guidance” as the scope boundary: everything materially related to that caption should be covered by evidence (automated or documented) in this change, and anything intentionally excluded should be called out with rationale and residual risk.

**Issue caption:** Snapshot tests: update workflow and CI guidance

### Domain context

This repository hosts the Fluxora streaming contract on Soroban (Stellar). Streams lock funded assets in contract
storage, release entitlement over time according to schedule parameters (including cliffs), and expose explicit roles:
senders who fund and may adjust certain parameters, recipients who claim vested value, administrators who may intervene
under policy, and—where applicable—permissionless callers for maintenance reads or cleanup. Integrators (wallets,
indexers, treasury tooling) depend on coherent state transitions, emitted signals, and error behavior when inputs or
timing are invalid.

### Work to complete

1. Characterize the **intended protocol semantics** for *Snapshot tests: update workflow and CI guidance* in both success and failure cases, treating the **Summary** above as the authoritative scope statement.
2. Map which roles may participate, what each role must prove, and which operations must be impossible for non-authorized actors.
3. Enumerate edge cases driven by time (start, cliff, end, cancellation freeze), numeric ranges, and stream status combinations; capture expectations with tests or explicit audit exceptions.
4. Ensure externally visible behavior—persistent fields, contract errors, and emitted payloads—does not contradict documentation that integrators rely on.

### Definition of done

- An independent reader can confirm *Snapshot tests: update workflow and CI guidance* matches the stated intent using tests, traces, snapshots (where used), or written review notes.
- Documentation for this slice of the protocol is consistent with observable on-chain behavior after the change.
- The pull request explains verification performed, scope boundaries, and any residual assumptions or risks.

### Constraints for contributors

- Describe **outcomes**, **invariants**, and **evidence**, not a single “right” internal design unless the issue title already names a concrete subsystem.
- Prefer **observable** guarantees: state transitions, balances, authorization failures, emitted events, error classifications, and documentation that integrators rely on.
- If something cannot be tested automatically, capture the gap as **audit notes** with explicit rationale and residual risk.

## Requirements and context

- Must be **secure**, **well-tested**, and **documented**.
- Should be **gas-conscious** (Soroban budget) and **easy to audit**.

## Suggested execution

1. Fork the repo and create a branch:
   ```bash
   git checkout -b feature/snapshot-workflow
   ```
2. Implement changes
   - **Contract / modules:** `contracts/stream/src/lib.rs`
   - **Comprehensive tests:** `contracts/stream/src/test.rs` and `contracts/stream/tests/integration_suite.rs`
   - **Documentation:** `docs/` (e.g. `streaming.md`, `security.md`, `events.md`)
   - Include **Rust doc comments** on public items (NatSpec-style clarity for auditors)
   - **Validate security assumptions** (CEI ordering, token trust model, auth paths)
3. **Test and commit**
   - Run `cargo test` in `contracts/stream` (and workspace if applicable)
   - Cover **edge cases** (overflow, TTL, paused/cancelled/completed)
   - Attach **test output** summary + **security notes** in the PR

### Example commit message

```
docs(testing): snapshot update workflow
```

## Guidelines

- Aim for **≥95%** coverage on touched Rust modules (`cargo llvm-cov` or project-standard tooling).
- **Clear documentation** (protocol semantics + operator runbooks where relevant)
- **Timeframe:** 96 hours

++++++

---
type: Feature
title: docs/streaming.md: protocol narrative vs code
labels: contracts, soroban, docs
assignees: ''
---

## Description

### Summary

Within the Fluxora streaming contract, this work tightens externally visible assurances for one focused concern. Treasury operators, recipient-facing applications, and third-party auditors must be able to reason about this area using only on-chain observables and published protocol documentation—without inferring hidden rules from how the implementation happens to be structured. The scenarios captured by this issue title must therefore have crisp success semantics, crisp failure semantics, and no silent drift between stored state, user-visible errors, and emitted events. Emitted signals and error classifications must remain coherent for indexers and wallets building on this protocol surface. In practical terms, treat “docs/streaming.md: protocol narrative vs code” as the scope boundary: everything materially related to that caption should be covered by evidence (automated or documented) in this change, and anything intentionally excluded should be called out with rationale and residual risk.

**Issue caption:** docs/streaming.md: protocol narrative vs code

### Domain context

This repository hosts the Fluxora streaming contract on Soroban (Stellar). Streams lock funded assets in contract
storage, release entitlement over time according to schedule parameters (including cliffs), and expose explicit roles:
senders who fund and may adjust certain parameters, recipients who claim vested value, administrators who may intervene
under policy, and—where applicable—permissionless callers for maintenance reads or cleanup. Integrators (wallets,
indexers, treasury tooling) depend on coherent state transitions, emitted signals, and error behavior when inputs or
timing are invalid.

### Work to complete

1. Characterize the **intended protocol semantics** for *docs/streaming.md: protocol narrative vs code* in both success and failure cases, treating the **Summary** above as the authoritative scope statement.
2. Map which roles may participate, what each role must prove, and which operations must be impossible for non-authorized actors.
3. Enumerate edge cases driven by time (start, cliff, end, cancellation freeze), numeric ranges, and stream status combinations; capture expectations with tests or explicit audit exceptions.
4. Ensure externally visible behavior—persistent fields, contract errors, and emitted payloads—does not contradict documentation that integrators rely on.

### Definition of done

- An independent reader can confirm *docs/streaming.md: protocol narrative vs code* matches the stated intent using tests, traces, snapshots (where used), or written review notes.
- Documentation for this slice of the protocol is consistent with observable on-chain behavior after the change.
- The pull request explains verification performed, scope boundaries, and any residual assumptions or risks.

### Constraints for contributors

- Describe **outcomes**, **invariants**, and **evidence**, not a single “right” internal design unless the issue title already names a concrete subsystem.
- Prefer **observable** guarantees: state transitions, balances, authorization failures, emitted events, error classifications, and documentation that integrators rely on.
- If something cannot be tested automatically, capture the gap as **audit notes** with explicit rationale and residual risk.

## Requirements and context

- Must be **secure**, **well-tested**, and **documented**.
- Should be **gas-conscious** (Soroban budget) and **easy to audit**.

## Suggested execution

1. Fork the repo and create a branch:
   ```bash
   git checkout -b feature/docs-streaming-sync
   ```
2. Implement changes
   - **Contract / modules:** `contracts/stream/src/lib.rs`
   - **Comprehensive tests:** `contracts/stream/src/test.rs` and `contracts/stream/tests/integration_suite.rs`
   - **Documentation:** `docs/` (e.g. `streaming.md`, `security.md`, `events.md`)
   - Include **Rust doc comments** on public items (NatSpec-style clarity for auditors)
   - **Validate security assumptions** (CEI ordering, token trust model, auth paths)
3. **Test and commit**
   - Run `cargo test` in `contracts/stream` (and workspace if applicable)
   - Cover **edge cases** (overflow, TTL, paused/cancelled/completed)
   - Attach **test output** summary + **security notes** in the PR

### Example commit message

```
docs(streaming): reconcile protocol documentation
```

## Guidelines

- Aim for **≥95%** coverage on touched Rust modules (`cargo llvm-cov` or project-standard tooling).
- **Clear documentation** (protocol semantics + operator runbooks where relevant)
- **Timeframe:** 96 hours

++++++

---
type: Feature
title: docs/security.md: threat model + admin powers
labels: contracts, soroban, docs, security
assignees: ''
---

## Description

### Summary

Within the Fluxora streaming contract, this work tightens externally visible assurances for one focused concern. Treasury operators, recipient-facing applications, and third-party auditors must be able to reason about this area using only on-chain observables and published protocol documentation—without inferring hidden rules from how the implementation happens to be structured. The scenarios captured by this issue title must therefore have crisp success semantics, crisp failure semantics, and no silent drift between stored state, user-visible errors, and emitted events. Permissionless maintenance or read paths, if involved, must have clearly documented trust impact for operators. In practical terms, treat “docs/security.md: threat model + admin powers” as the scope boundary: everything materially related to that caption should be covered by evidence (automated or documented) in this change, and anything intentionally excluded should be called out with rationale and residual risk.

**Issue caption:** docs/security.md: threat model + admin powers

### Domain context

This repository hosts the Fluxora streaming contract on Soroban (Stellar). Streams lock funded assets in contract
storage, release entitlement over time according to schedule parameters (including cliffs), and expose explicit roles:
senders who fund and may adjust certain parameters, recipients who claim vested value, administrators who may intervene
under policy, and—where applicable—permissionless callers for maintenance reads or cleanup. Integrators (wallets,
indexers, treasury tooling) depend on coherent state transitions, emitted signals, and error behavior when inputs or
timing are invalid.

### Work to complete

1. Characterize the **intended protocol semantics** for *docs/security.md: threat model + admin powers* in both success and failure cases, treating the **Summary** above as the authoritative scope statement.
2. Map which roles may participate, what each role must prove, and which operations must be impossible for non-authorized actors.
3. Enumerate edge cases driven by time (start, cliff, end, cancellation freeze), numeric ranges, and stream status combinations; capture expectations with tests or explicit audit exceptions.
4. Ensure externally visible behavior—persistent fields, contract errors, and emitted payloads—does not contradict documentation that integrators rely on.

### Definition of done

- An independent reader can confirm *docs/security.md: threat model + admin powers* matches the stated intent using tests, traces, snapshots (where used), or written review notes.
- Documentation for this slice of the protocol is consistent with observable on-chain behavior after the change.
- The pull request explains verification performed, scope boundaries, and any residual assumptions or risks.

### Constraints for contributors

- Describe **outcomes**, **invariants**, and **evidence**, not a single “right” internal design unless the issue title already names a concrete subsystem.
- Prefer **observable** guarantees: state transitions, balances, authorization failures, emitted events, error classifications, and documentation that integrators rely on.
- If something cannot be tested automatically, capture the gap as **audit notes** with explicit rationale and residual risk.

## Requirements and context

- Must be **secure**, **well-tested**, and **documented**.
- Should be **gas-conscious** (Soroban budget) and **easy to audit**.

## Suggested execution

1. Fork the repo and create a branch:
   ```bash
   git checkout -b feature/docs-security-threat-model
   ```
2. Implement changes
   - **Contract / modules:** `contracts/stream/src/lib.rs`
   - **Comprehensive tests:** `contracts/stream/src/test.rs` and `contracts/stream/tests/integration_suite.rs`
   - **Documentation:** `docs/` (e.g. `streaming.md`, `security.md`, `events.md`)
   - Include **Rust doc comments** on public items (NatSpec-style clarity for auditors)
   - **Validate security assumptions** (CEI ordering, token trust model, auth paths)
3. **Test and commit**
   - Run `cargo test` in `contracts/stream` (and workspace if applicable)
   - Cover **edge cases** (overflow, TTL, paused/cancelled/completed)
   - Attach **test output** summary + **security notes** in the PR

### Example commit message

```
docs(security): expand threat model
```

## Guidelines

- Aim for **≥95%** coverage on touched Rust modules (`cargo llvm-cov` or project-standard tooling).
- **Clear documentation** (protocol semantics + operator runbooks where relevant)
- **Timeframe:** 96 hours

++++++

---
type: Task
title: docs/mainnet.md: deployment checklist alignment
labels: contracts, soroban, docs, ops
assignees: ''
---

## Description

### Summary

Within the Fluxora streaming contract, this work tightens externally visible assurances for one focused concern. Treasury operators, recipient-facing applications, and third-party auditors must be able to reason about this area using only on-chain observables and published protocol documentation—without inferring hidden rules from how the implementation happens to be structured. The scenarios captured by this issue title must therefore have crisp success semantics, crisp failure semantics, and no silent drift between stored state, user-visible errors, and emitted events. Time boundaries (start, cliff, end, cancellation freeze) and numeric ranges must be enumerated so entitlement cannot be misread under edge timing. In practical terms, treat “docs/mainnet.md: deployment checklist alignment” as the scope boundary: everything materially related to that caption should be covered by evidence (automated or documented) in this change, and anything intentionally excluded should be called out with rationale and residual risk.

**Issue caption:** docs/mainnet.md: deployment checklist alignment

### Domain context

This repository hosts the Fluxora streaming contract on Soroban (Stellar). Streams lock funded assets in contract
storage, release entitlement over time according to schedule parameters (including cliffs), and expose explicit roles:
senders who fund and may adjust certain parameters, recipients who claim vested value, administrators who may intervene
under policy, and—where applicable—permissionless callers for maintenance reads or cleanup. Integrators (wallets,
indexers, treasury tooling) depend on coherent state transitions, emitted signals, and error behavior when inputs or
timing are invalid.

### Work to complete

1. Characterize the **intended protocol semantics** for *docs/mainnet.md: deployment checklist alignment* in both success and failure cases, treating the **Summary** above as the authoritative scope statement.
2. Map which roles may participate, what each role must prove, and which operations must be impossible for non-authorized actors.
3. Enumerate edge cases driven by time (start, cliff, end, cancellation freeze), numeric ranges, and stream status combinations; capture expectations with tests or explicit audit exceptions.
4. Ensure externally visible behavior—persistent fields, contract errors, and emitted payloads—does not contradict documentation that integrators rely on.

### Definition of done

- An independent reader can confirm *docs/mainnet.md: deployment checklist alignment* matches the stated intent using tests, traces, snapshots (where used), or written review notes.
- Documentation for this slice of the protocol is consistent with observable on-chain behavior after the change.
- The pull request explains verification performed, scope boundaries, and any residual assumptions or risks.

### Constraints for contributors

- Describe **outcomes**, **invariants**, and **evidence**, not a single “right” internal design unless the issue title already names a concrete subsystem.
- Prefer **observable** guarantees: state transitions, balances, authorization failures, emitted events, error classifications, and documentation that integrators rely on.
- If something cannot be tested automatically, capture the gap as **audit notes** with explicit rationale and residual risk.

## Requirements and context

- Must be **secure**, **well-tested**, and **documented**.
- Should be **gas-conscious** (Soroban budget) and **easy to audit**.

## Suggested execution

1. Fork the repo and create a branch:
   ```bash
   git checkout -b feature/docs-mainnet-checklist
   ```
2. Implement changes
   - **Contract / modules:** `contracts/stream/src/lib.rs`
   - **Comprehensive tests:** `contracts/stream/src/test.rs` and `contracts/stream/tests/integration_suite.rs`
   - **Documentation:** `docs/` (e.g. `streaming.md`, `security.md`, `events.md`)
   - Include **Rust doc comments** on public items (NatSpec-style clarity for auditors)
   - **Validate security assumptions** (CEI ordering, token trust model, auth paths)
3. **Test and commit**
   - Run `cargo test` in `contracts/stream` (and workspace if applicable)
   - Cover **edge cases** (overflow, TTL, paused/cancelled/completed)
   - Attach **test output** summary + **security notes** in the PR

### Example commit message

```
docs(mainnet): align deployment checklist
```

## Guidelines

- Aim for **≥95%** coverage on touched Rust modules (`cargo llvm-cov` or project-standard tooling).
- **Clear documentation** (protocol semantics + operator runbooks where relevant)
- **Timeframe:** 96 hours

++++++

---
type: Feature
title: Gas / budget review: hot paths and batching
labels: contracts, soroban, performance
assignees: ''
---

## Description

### Summary

Within the Fluxora streaming contract, this work tightens externally visible assurances for one focused concern. Treasury operators, recipient-facing applications, and third-party auditors must be able to reason about this area using only on-chain observables and published protocol documentation—without inferring hidden rules from how the implementation happens to be structured. The scenarios captured by this issue title must therefore have crisp success semantics, crisp failure semantics, and no silent drift between stored state, user-visible errors, and emitted events. Time boundaries (start, cliff, end, cancellation freeze) and numeric ranges must be enumerated so entitlement cannot be misread under edge timing. In practical terms, treat “Gas / budget review: hot paths and batching” as the scope boundary: everything materially related to that caption should be covered by evidence (automated or documented) in this change, and anything intentionally excluded should be called out with rationale and residual risk.

**Issue caption:** Gas / budget review: hot paths and batching

### Domain context

This repository hosts the Fluxora streaming contract on Soroban (Stellar). Streams lock funded assets in contract
storage, release entitlement over time according to schedule parameters (including cliffs), and expose explicit roles:
senders who fund and may adjust certain parameters, recipients who claim vested value, administrators who may intervene
under policy, and—where applicable—permissionless callers for maintenance reads or cleanup. Integrators (wallets,
indexers, treasury tooling) depend on coherent state transitions, emitted signals, and error behavior when inputs or
timing are invalid.

### Work to complete

1. Characterize the **intended protocol semantics** for *Gas / budget review: hot paths and batching* in both success and failure cases, treating the **Summary** above as the authoritative scope statement.
2. Map which roles may participate, what each role must prove, and which operations must be impossible for non-authorized actors.
3. Enumerate edge cases driven by time (start, cliff, end, cancellation freeze), numeric ranges, and stream status combinations; capture expectations with tests or explicit audit exceptions.
4. Ensure externally visible behavior—persistent fields, contract errors, and emitted payloads—does not contradict documentation that integrators rely on.

### Definition of done

- An independent reader can confirm *Gas / budget review: hot paths and batching* matches the stated intent using tests, traces, snapshots (where used), or written review notes.
- Documentation for this slice of the protocol is consistent with observable on-chain behavior after the change.
- The pull request explains verification performed, scope boundaries, and any residual assumptions or risks.

### Constraints for contributors

- Describe **outcomes**, **invariants**, and **evidence**, not a single “right” internal design unless the issue title already names a concrete subsystem.
- Prefer **observable** guarantees: state transitions, balances, authorization failures, emitted events, error classifications, and documentation that integrators rely on.
- If something cannot be tested automatically, capture the gap as **audit notes** with explicit rationale and residual risk.

## Requirements and context

- Must be **secure**, **well-tested**, and **documented**.
- Should be **gas-conscious** (Soroban budget) and **easy to audit**.

## Suggested execution

1. Fork the repo and create a branch:
   ```bash
   git checkout -b feature/gas-review
   ```
2. Implement changes
   - **Contract / modules:** `contracts/stream/src/lib.rs`
   - **Comprehensive tests:** `contracts/stream/src/test.rs` and `contracts/stream/tests/integration_suite.rs`
   - **Documentation:** `docs/` (e.g. `streaming.md`, `security.md`, `events.md`)
   - Include **Rust doc comments** on public items (NatSpec-style clarity for auditors)
   - **Validate security assumptions** (CEI ordering, token trust model, auth paths)
3. **Test and commit**
   - Run `cargo test` in `contracts/stream` (and workspace if applicable)
   - Cover **edge cases** (overflow, TTL, paused/cancelled/completed)
   - Attach **test output** summary + **security notes** in the PR

### Example commit message

```
docs(perf): note resource usage hotspots
```

## Guidelines

- Aim for **≥95%** coverage on touched Rust modules (`cargo llvm-cov` or project-standard tooling).
- **Clear documentation** (protocol semantics + operator runbooks where relevant)
- **Timeframe:** 96 hours

++++++

---
type: Feature
title: Formal invariants checklist (pre-audit)
labels: contracts, soroban, audit
assignees: ''
---

## Description

### Summary

Within the Fluxora streaming contract, this work tightens externally visible assurances for one focused concern. Treasury operators, recipient-facing applications, and third-party auditors must be able to reason about this area using only on-chain observables and published protocol documentation—without inferring hidden rules from how the implementation happens to be structured. The scenarios captured by this issue title must therefore have crisp success semantics, crisp failure semantics, and no silent drift between stored state, user-visible errors, and emitted events. Emitted signals and error classifications must remain coherent for indexers and wallets building on this protocol surface. In practical terms, treat “Formal invariants checklist (pre-audit)” as the scope boundary: everything materially related to that caption should be covered by evidence (automated or documented) in this change, and anything intentionally excluded should be called out with rationale and residual risk.

**Issue caption:** Formal invariants checklist (pre-audit)

### Domain context

This repository hosts the Fluxora streaming contract on Soroban (Stellar). Streams lock funded assets in contract
storage, release entitlement over time according to schedule parameters (including cliffs), and expose explicit roles:
senders who fund and may adjust certain parameters, recipients who claim vested value, administrators who may intervene
under policy, and—where applicable—permissionless callers for maintenance reads or cleanup. Integrators (wallets,
indexers, treasury tooling) depend on coherent state transitions, emitted signals, and error behavior when inputs or
timing are invalid.

### Work to complete

1. Characterize the **intended protocol semantics** for *Formal invariants checklist (pre-audit)* in both success and failure cases, treating the **Summary** above as the authoritative scope statement.
2. Map which roles may participate, what each role must prove, and which operations must be impossible for non-authorized actors.
3. Enumerate edge cases driven by time (start, cliff, end, cancellation freeze), numeric ranges, and stream status combinations; capture expectations with tests or explicit audit exceptions.
4. Ensure externally visible behavior—persistent fields, contract errors, and emitted payloads—does not contradict documentation that integrators rely on.

### Definition of done

- An independent reader can confirm *Formal invariants checklist (pre-audit)* matches the stated intent using tests, traces, snapshots (where used), or written review notes.
- Documentation for this slice of the protocol is consistent with observable on-chain behavior after the change.
- The pull request explains verification performed, scope boundaries, and any residual assumptions or risks.

### Constraints for contributors

- Describe **outcomes**, **invariants**, and **evidence**, not a single “right” internal design unless the issue title already names a concrete subsystem.
- Prefer **observable** guarantees: state transitions, balances, authorization failures, emitted events, error classifications, and documentation that integrators rely on.
- If something cannot be tested automatically, capture the gap as **audit notes** with explicit rationale and residual risk.

## Requirements and context

- Must be **secure**, **well-tested**, and **documented**.
- Should be **gas-conscious** (Soroban budget) and **easy to audit**.

## Suggested execution

1. Fork the repo and create a branch:
   ```bash
   git checkout -b feature/audit-invariants
   ```
2. Implement changes
   - **Contract / modules:** `contracts/stream/src/lib.rs`
   - **Comprehensive tests:** `contracts/stream/src/test.rs` and `contracts/stream/tests/integration_suite.rs`
   - **Documentation:** `docs/` (e.g. `streaming.md`, `security.md`, `events.md`)
   - Include **Rust doc comments** on public items (NatSpec-style clarity for auditors)
   - **Validate security assumptions** (CEI ordering, token trust model, auth paths)
3. **Test and commit**
   - Run `cargo test` in `contracts/stream` (and workspace if applicable)
   - Cover **edge cases** (overflow, TTL, paused/cancelled/completed)
   - Attach **test output** summary + **security notes** in the PR

### Example commit message

```
docs(audit): add invariant checklist
```

## Guidelines

- Aim for **≥95%** coverage on touched Rust modules (`cargo llvm-cov` or project-standard tooling).
- **Clear documentation** (protocol semantics + operator runbooks where relevant)
- **Timeframe:** 96 hours

++++++

---
type: Feature
title: Fuzz harness for accrual.rs (cargo-fuzz optional)
labels: contracts, soroban, testing, fuzzing
assignees: ''
---

## Description

### Summary

Within the Fluxora streaming contract, this work tightens externally visible assurances for one focused concern. Treasury operators, recipient-facing applications, and third-party auditors must be able to reason about this area using only on-chain observables and published protocol documentation—without inferring hidden rules from how the implementation happens to be structured. The scenarios captured by this issue title must therefore have crisp success semantics, crisp failure semantics, and no silent drift between stored state, user-visible errors, and emitted events. Time boundaries (start, cliff, end, cancellation freeze) and numeric ranges must be enumerated so entitlement cannot be misread under edge timing. In practical terms, treat “Fuzz harness for accrual.rs (cargo-fuzz optional)” as the scope boundary: everything materially related to that caption should be covered by evidence (automated or documented) in this change, and anything intentionally excluded should be called out with rationale and residual risk.

**Issue caption:** Fuzz harness for accrual.rs (cargo-fuzz optional)

### Domain context

This repository hosts the Fluxora streaming contract on Soroban (Stellar). Streams lock funded assets in contract
storage, release entitlement over time according to schedule parameters (including cliffs), and expose explicit roles:
senders who fund and may adjust certain parameters, recipients who claim vested value, administrators who may intervene
under policy, and—where applicable—permissionless callers for maintenance reads or cleanup. Integrators (wallets,
indexers, treasury tooling) depend on coherent state transitions, emitted signals, and error behavior when inputs or
timing are invalid.

### Work to complete

1. Characterize the **intended protocol semantics** for *Fuzz harness for accrual.rs (cargo-fuzz optional)* in both success and failure cases, treating the **Summary** above as the authoritative scope statement.
2. Map which roles may participate, what each role must prove, and which operations must be impossible for non-authorized actors.
3. Enumerate edge cases driven by time (start, cliff, end, cancellation freeze), numeric ranges, and stream status combinations; capture expectations with tests or explicit audit exceptions.
4. Ensure externally visible behavior—persistent fields, contract errors, and emitted payloads—does not contradict documentation that integrators rely on.

### Definition of done

- An independent reader can confirm *Fuzz harness for accrual.rs (cargo-fuzz optional)* matches the stated intent using tests, traces, snapshots (where used), or written review notes.
- Documentation for this slice of the protocol is consistent with observable on-chain behavior after the change.
- The pull request explains verification performed, scope boundaries, and any residual assumptions or risks.

### Constraints for contributors

- Describe **outcomes**, **invariants**, and **evidence**, not a single “right” internal design unless the issue title already names a concrete subsystem.
- Prefer **observable** guarantees: state transitions, balances, authorization failures, emitted events, error classifications, and documentation that integrators rely on.
- If something cannot be tested automatically, capture the gap as **audit notes** with explicit rationale and residual risk.

## Requirements and context

- Must be **secure**, **well-tested**, and **documented**.
- Should be **gas-conscious** (Soroban budget) and **easy to audit**.

## Suggested execution

1. Fork the repo and create a branch:
   ```bash
   git checkout -b feature/fuzz-accrual
   ```
2. Implement changes
   - **Contract / modules:** `contracts/stream/src/lib.rs`
   - **Comprehensive tests:** `contracts/stream/src/test.rs` and `contracts/stream/tests/integration_suite.rs`
   - **Documentation:** `docs/` (e.g. `streaming.md`, `security.md`, `events.md`)
   - Include **Rust doc comments** on public items (NatSpec-style clarity for auditors)
   - **Validate security assumptions** (CEI ordering, token trust model, auth paths)
3. **Test and commit**
   - Run `cargo test` in `contracts/stream` (and workspace if applicable)
   - Cover **edge cases** (overflow, TTL, paused/cancelled/completed)
   - Attach **test output** summary + **security notes** in the PR

### Example commit message

```
test(accrual): add fuzz harness scaffolding
```

## Guidelines

- Aim for **≥95%** coverage on touched Rust modules (`cargo llvm-cov` or project-standard tooling).
- **Clear documentation** (protocol semantics + operator runbooks where relevant)
- **Timeframe:** 120 hours

++++++

---
type: Feature
title: Reproducible WASM builds: verify checksums in CI
labels: contracts, soroban, ci
assignees: ''
---

## Description

### Summary

Within the Fluxora streaming contract, this work tightens externally visible assurances for one focused concern. Treasury operators, recipient-facing applications, and third-party auditors must be able to reason about this area using only on-chain observables and published protocol documentation—without inferring hidden rules from how the implementation happens to be structured. The scenarios captured by this issue title must therefore have crisp success semantics, crisp failure semantics, and no silent drift between stored state, user-visible errors, and emitted events. Time boundaries (start, cliff, end, cancellation freeze) and numeric ranges must be enumerated so entitlement cannot be misread under edge timing. In practical terms, treat “Reproducible WASM builds: verify checksums in CI” as the scope boundary: everything materially related to that caption should be covered by evidence (automated or documented) in this change, and anything intentionally excluded should be called out with rationale and residual risk.

**Issue caption:** Reproducible WASM builds: verify checksums in CI

### Domain context

This repository hosts the Fluxora streaming contract on Soroban (Stellar). Streams lock funded assets in contract
storage, release entitlement over time according to schedule parameters (including cliffs), and expose explicit roles:
senders who fund and may adjust certain parameters, recipients who claim vested value, administrators who may intervene
under policy, and—where applicable—permissionless callers for maintenance reads or cleanup. Integrators (wallets,
indexers, treasury tooling) depend on coherent state transitions, emitted signals, and error behavior when inputs or
timing are invalid.

### Work to complete

1. Characterize the **intended protocol semantics** for *Reproducible WASM builds: verify checksums in CI* in both success and failure cases, treating the **Summary** above as the authoritative scope statement.
2. Map which roles may participate, what each role must prove, and which operations must be impossible for non-authorized actors.
3. Enumerate edge cases driven by time (start, cliff, end, cancellation freeze), numeric ranges, and stream status combinations; capture expectations with tests or explicit audit exceptions.
4. Ensure externally visible behavior—persistent fields, contract errors, and emitted payloads—does not contradict documentation that integrators rely on.

### Definition of done

- An independent reader can confirm *Reproducible WASM builds: verify checksums in CI* matches the stated intent using tests, traces, snapshots (where used), or written review notes.
- Documentation for this slice of the protocol is consistent with observable on-chain behavior after the change.
- The pull request explains verification performed, scope boundaries, and any residual assumptions or risks.

### Constraints for contributors

- Describe **outcomes**, **invariants**, and **evidence**, not a single “right” internal design unless the issue title already names a concrete subsystem.
- Prefer **observable** guarantees: state transitions, balances, authorization failures, emitted events, error classifications, and documentation that integrators rely on.
- If something cannot be tested automatically, capture the gap as **audit notes** with explicit rationale and residual risk.

## Requirements and context

- Must be **secure**, **well-tested**, and **documented**.
- Should be **gas-conscious** (Soroban budget) and **easy to audit**.

## Suggested execution

1. Fork the repo and create a branch:
   ```bash
   git checkout -b feature/ci-wasm-checksum
   ```
2. Implement changes
   - **Contract / modules:** `contracts/stream/src/lib.rs`
   - **Comprehensive tests:** `contracts/stream/src/test.rs` and `contracts/stream/tests/integration_suite.rs`
   - **Documentation:** `docs/` (e.g. `streaming.md`, `security.md`, `events.md`)
   - Include **Rust doc comments** on public items (NatSpec-style clarity for auditors)
   - **Validate security assumptions** (CEI ordering, token trust model, auth paths)
3. **Test and commit**
   - Run `cargo test` in `contracts/stream` (and workspace if applicable)
   - Cover **edge cases** (overflow, TTL, paused/cancelled/completed)
   - Attach **test output** summary + **security notes** in the PR

### Example commit message

```
ci: verify wasm build reproducibility
```

## Guidelines

- Aim for **≥95%** coverage on touched Rust modules (`cargo llvm-cov` or project-standard tooling).
- **Clear documentation** (protocol semantics + operator runbooks where relevant)
- **Timeframe:** 96 hours

++++++

---
type: Task
title: Dependency audit: cargo-deny / advisory scanning
labels: contracts, soroban, security, ci
assignees: ''
---

## Description

### Summary

Within the Fluxora streaming contract, this work tightens externally visible assurances for one focused concern. Treasury operators, recipient-facing applications, and third-party auditors must be able to reason about this area using only on-chain observables and published protocol documentation—without inferring hidden rules from how the implementation happens to be structured. The scenarios captured by this issue title must therefore have crisp success semantics, crisp failure semantics, and no silent drift between stored state, user-visible errors, and emitted events. Emitted signals and error classifications must remain coherent for indexers and wallets building on this protocol surface. In practical terms, treat “Dependency audit: cargo-deny / advisory scanning” as the scope boundary: everything materially related to that caption should be covered by evidence (automated or documented) in this change, and anything intentionally excluded should be called out with rationale and residual risk.

**Issue caption:** Dependency audit: cargo-deny / advisory scanning

### Domain context

This repository hosts the Fluxora streaming contract on Soroban (Stellar). Streams lock funded assets in contract
storage, release entitlement over time according to schedule parameters (including cliffs), and expose explicit roles:
senders who fund and may adjust certain parameters, recipients who claim vested value, administrators who may intervene
under policy, and—where applicable—permissionless callers for maintenance reads or cleanup. Integrators (wallets,
indexers, treasury tooling) depend on coherent state transitions, emitted signals, and error behavior when inputs or
timing are invalid.

### Work to complete

1. Characterize the **intended protocol semantics** for *Dependency audit: cargo-deny / advisory scanning* in both success and failure cases, treating the **Summary** above as the authoritative scope statement.
2. Map which roles may participate, what each role must prove, and which operations must be impossible for non-authorized actors.
3. Enumerate edge cases driven by time (start, cliff, end, cancellation freeze), numeric ranges, and stream status combinations; capture expectations with tests or explicit audit exceptions.
4. Ensure externally visible behavior—persistent fields, contract errors, and emitted payloads—does not contradict documentation that integrators rely on.

### Definition of done

- An independent reader can confirm *Dependency audit: cargo-deny / advisory scanning* matches the stated intent using tests, traces, snapshots (where used), or written review notes.
- Documentation for this slice of the protocol is consistent with observable on-chain behavior after the change.
- The pull request explains verification performed, scope boundaries, and any residual assumptions or risks.

### Constraints for contributors

- Describe **outcomes**, **invariants**, and **evidence**, not a single “right” internal design unless the issue title already names a concrete subsystem.
- Prefer **observable** guarantees: state transitions, balances, authorization failures, emitted events, error classifications, and documentation that integrators rely on.
- If something cannot be tested automatically, capture the gap as **audit notes** with explicit rationale and residual risk.

## Requirements and context

- Must be **secure**, **well-tested**, and **documented**.
- Should be **gas-conscious** (Soroban budget) and **easy to audit**.

## Suggested execution

1. Fork the repo and create a branch:
   ```bash
   git checkout -b feature/cargo-deny
   ```
2. Implement changes
   - **Contract / modules:** `contracts/stream/src/lib.rs`
   - **Comprehensive tests:** `contracts/stream/src/test.rs` and `contracts/stream/tests/integration_suite.rs`
   - **Documentation:** `docs/` (e.g. `streaming.md`, `security.md`, `events.md`)
   - Include **Rust doc comments** on public items (NatSpec-style clarity for auditors)
   - **Validate security assumptions** (CEI ordering, token trust model, auth paths)
3. **Test and commit**
   - Run `cargo test` in `contracts/stream` (and workspace if applicable)
   - Cover **edge cases** (overflow, TTL, paused/cancelled/completed)
   - Attach **test output** summary + **security notes** in the PR

### Example commit message

```
ci: add cargo deny policy
```

## Guidelines

- Aim for **≥95%** coverage on touched Rust modules (`cargo llvm-cov` or project-standard tooling).
- **Clear documentation** (protocol semantics + operator runbooks where relevant)
- **Timeframe:** 96 hours

++++++

---
type: Feature
title: Malicious token assumptions: document non-goals
labels: contracts, soroban, security, tokens
assignees: ''
---

## Description

### Summary

Within the Fluxora streaming contract, this work tightens externally visible assurances for one focused concern. Treasury operators, recipient-facing applications, and third-party auditors must be able to reason about this area using only on-chain observables and published protocol documentation—without inferring hidden rules from how the implementation happens to be structured. The scenarios captured by this issue title must therefore have crisp success semantics, crisp failure semantics, and no silent drift between stored state, user-visible errors, and emitted events. Permissionless maintenance or read paths, if involved, must have clearly documented trust impact for operators. In practical terms, treat “Malicious token assumptions: document non-goals” as the scope boundary: everything materially related to that caption should be covered by evidence (automated or documented) in this change, and anything intentionally excluded should be called out with rationale and residual risk.

**Issue caption:** Malicious token assumptions: document non-goals

### Domain context

This repository hosts the Fluxora streaming contract on Soroban (Stellar). Streams lock funded assets in contract
storage, release entitlement over time according to schedule parameters (including cliffs), and expose explicit roles:
senders who fund and may adjust certain parameters, recipients who claim vested value, administrators who may intervene
under policy, and—where applicable—permissionless callers for maintenance reads or cleanup. Integrators (wallets,
indexers, treasury tooling) depend on coherent state transitions, emitted signals, and error behavior when inputs or
timing are invalid.

### Work to complete

1. Characterize the **intended protocol semantics** for *Malicious token assumptions: document non-goals* in both success and failure cases, treating the **Summary** above as the authoritative scope statement.
2. Map which roles may participate, what each role must prove, and which operations must be impossible for non-authorized actors.
3. Enumerate edge cases driven by time (start, cliff, end, cancellation freeze), numeric ranges, and stream status combinations; capture expectations with tests or explicit audit exceptions.
4. Ensure externally visible behavior—persistent fields, contract errors, and emitted payloads—does not contradict documentation that integrators rely on.

### Definition of done

- An independent reader can confirm *Malicious token assumptions: document non-goals* matches the stated intent using tests, traces, snapshots (where used), or written review notes.
- Documentation for this slice of the protocol is consistent with observable on-chain behavior after the change.
- The pull request explains verification performed, scope boundaries, and any residual assumptions or risks.

### Constraints for contributors

- Describe **outcomes**, **invariants**, and **evidence**, not a single “right” internal design unless the issue title already names a concrete subsystem.
- Prefer **observable** guarantees: state transitions, balances, authorization failures, emitted events, error classifications, and documentation that integrators rely on.
- If something cannot be tested automatically, capture the gap as **audit notes** with explicit rationale and residual risk.

## Requirements and context

- Must be **secure**, **well-tested**, and **documented**.
- Should be **gas-conscious** (Soroban budget) and **easy to audit**.

## Suggested execution

1. Fork the repo and create a branch:
   ```bash
   git checkout -b feature/token-assumptions-doc
   ```
2. Implement changes
   - **Contract / modules:** `contracts/stream/src/lib.rs`
   - **Comprehensive tests:** `contracts/stream/src/test.rs` and `contracts/stream/tests/integration_suite.rs`
   - **Documentation:** `docs/` (e.g. `streaming.md`, `security.md`, `events.md`)
   - Include **Rust doc comments** on public items (NatSpec-style clarity for auditors)
   - **Validate security assumptions** (CEI ordering, token trust model, auth paths)
3. **Test and commit**
   - Run `cargo test` in `contracts/stream` (and workspace if applicable)
   - Cover **edge cases** (overflow, TTL, paused/cancelled/completed)
   - Attach **test output** summary + **security notes** in the PR

### Example commit message

```
docs(security): document token compatibility assumptions
```

## Guidelines

- Aim for **≥95%** coverage on touched Rust modules (`cargo llvm-cov` or project-standard tooling).
- **Clear documentation** (protocol semantics + operator runbooks where relevant)
- **Timeframe:** 96 hours

++++++

---
type: Feature
title: Negative tests: withdraw as sender/admin (must fail)
labels: contracts, soroban, testing, security
assignees: ''
---

## Description

### Summary

Within the Fluxora streaming contract, this work tightens externally visible assurances for one focused concern. Treasury operators, recipient-facing applications, and third-party auditors must be able to reason about this area using only on-chain observables and published protocol documentation—without inferring hidden rules from how the implementation happens to be structured. The scenarios captured by this issue title must therefore have crisp success semantics, crisp failure semantics, and no silent drift between stored state, user-visible errors, and emitted events. Permissionless maintenance or read paths, if involved, must have clearly documented trust impact for operators. In practical terms, treat “Negative tests: withdraw as sender/admin (must fail)” as the scope boundary: everything materially related to that caption should be covered by evidence (automated or documented) in this change, and anything intentionally excluded should be called out with rationale and residual risk.

**Issue caption:** Negative tests: withdraw as sender/admin (must fail)

### Domain context

This repository hosts the Fluxora streaming contract on Soroban (Stellar). Streams lock funded assets in contract
storage, release entitlement over time according to schedule parameters (including cliffs), and expose explicit roles:
senders who fund and may adjust certain parameters, recipients who claim vested value, administrators who may intervene
under policy, and—where applicable—permissionless callers for maintenance reads or cleanup. Integrators (wallets,
indexers, treasury tooling) depend on coherent state transitions, emitted signals, and error behavior when inputs or
timing are invalid.

### Work to complete

1. Characterize the **intended protocol semantics** for *Negative tests: withdraw as sender/admin (must fail)* in both success and failure cases, treating the **Summary** above as the authoritative scope statement.
2. Map which roles may participate, what each role must prove, and which operations must be impossible for non-authorized actors.
3. Enumerate edge cases driven by time (start, cliff, end, cancellation freeze), numeric ranges, and stream status combinations; capture expectations with tests or explicit audit exceptions.
4. Ensure externally visible behavior—persistent fields, contract errors, and emitted payloads—does not contradict documentation that integrators rely on.

### Definition of done

- An independent reader can confirm *Negative tests: withdraw as sender/admin (must fail)* matches the stated intent using tests, traces, snapshots (where used), or written review notes.
- Documentation for this slice of the protocol is consistent with observable on-chain behavior after the change.
- The pull request explains verification performed, scope boundaries, and any residual assumptions or risks.

### Constraints for contributors

- Describe **outcomes**, **invariants**, and **evidence**, not a single “right” internal design unless the issue title already names a concrete subsystem.
- Prefer **observable** guarantees: state transitions, balances, authorization failures, emitted events, error classifications, and documentation that integrators rely on.
- If something cannot be tested automatically, capture the gap as **audit notes** with explicit rationale and residual risk.

## Requirements and context

- Must be **secure**, **well-tested**, and **documented**.
- Should be **gas-conscious** (Soroban budget) and **easy to audit**.

## Suggested execution

1. Fork the repo and create a branch:
   ```bash
   git checkout -b feature/negative-withdraw-auth
   ```
2. Implement changes
   - **Contract / modules:** `contracts/stream/src/lib.rs`
   - **Comprehensive tests:** `contracts/stream/src/test.rs` and `contracts/stream/tests/integration_suite.rs`
   - **Documentation:** `docs/` (e.g. `streaming.md`, `security.md`, `events.md`)
   - Include **Rust doc comments** on public items (NatSpec-style clarity for auditors)
   - **Validate security assumptions** (CEI ordering, token trust model, auth paths)
3. **Test and commit**
   - Run `cargo test` in `contracts/stream` (and workspace if applicable)
   - Cover **edge cases** (overflow, TTL, paused/cancelled/completed)
   - Attach **test output** summary + **security notes** in the PR

### Example commit message

```
test(stream): negative withdraw authorization
```

## Guidelines

- Aim for **≥95%** coverage on touched Rust modules (`cargo llvm-cov` or project-standard tooling).
- **Clear documentation** (protocol semantics + operator runbooks where relevant)
- **Timeframe:** 96 hours

++++++

---
type: Feature
title: Negative tests: pause/resume by non-sender/non-admin
labels: contracts, soroban, testing, security
assignees: ''
---

## Description

### Summary

Within the Fluxora streaming contract, this work tightens externally visible assurances for one focused concern. Treasury operators, recipient-facing applications, and third-party auditors must be able to reason about this area using only on-chain observables and published protocol documentation—without inferring hidden rules from how the implementation happens to be structured. The scenarios captured by this issue title must therefore have crisp success semantics, crisp failure semantics, and no silent drift between stored state, user-visible errors, and emitted events. Emitted signals and error classifications must remain coherent for indexers and wallets building on this protocol surface. In practical terms, treat “Negative tests: pause/resume by non-sender/non-admin” as the scope boundary: everything materially related to that caption should be covered by evidence (automated or documented) in this change, and anything intentionally excluded should be called out with rationale and residual risk.

**Issue caption:** Negative tests: pause/resume by non-sender/non-admin

### Domain context

This repository hosts the Fluxora streaming contract on Soroban (Stellar). Streams lock funded assets in contract
storage, release entitlement over time according to schedule parameters (including cliffs), and expose explicit roles:
senders who fund and may adjust certain parameters, recipients who claim vested value, administrators who may intervene
under policy, and—where applicable—permissionless callers for maintenance reads or cleanup. Integrators (wallets,
indexers, treasury tooling) depend on coherent state transitions, emitted signals, and error behavior when inputs or
timing are invalid.

### Work to complete

1. Characterize the **intended protocol semantics** for *Negative tests: pause/resume by non-sender/non-admin* in both success and failure cases, treating the **Summary** above as the authoritative scope statement.
2. Map which roles may participate, what each role must prove, and which operations must be impossible for non-authorized actors.
3. Enumerate edge cases driven by time (start, cliff, end, cancellation freeze), numeric ranges, and stream status combinations; capture expectations with tests or explicit audit exceptions.
4. Ensure externally visible behavior—persistent fields, contract errors, and emitted payloads—does not contradict documentation that integrators rely on.

### Definition of done

- An independent reader can confirm *Negative tests: pause/resume by non-sender/non-admin* matches the stated intent using tests, traces, snapshots (where used), or written review notes.
- Documentation for this slice of the protocol is consistent with observable on-chain behavior after the change.
- The pull request explains verification performed, scope boundaries, and any residual assumptions or risks.

### Constraints for contributors

- Describe **outcomes**, **invariants**, and **evidence**, not a single “right” internal design unless the issue title already names a concrete subsystem.
- Prefer **observable** guarantees: state transitions, balances, authorization failures, emitted events, error classifications, and documentation that integrators rely on.
- If something cannot be tested automatically, capture the gap as **audit notes** with explicit rationale and residual risk.

## Requirements and context

- Must be **secure**, **well-tested**, and **documented**.
- Should be **gas-conscious** (Soroban budget) and **easy to audit**.

## Suggested execution

1. Fork the repo and create a branch:
   ```bash
   git checkout -b feature/negative-lifecycle-auth
   ```
2. Implement changes
   - **Contract / modules:** `contracts/stream/src/lib.rs`
   - **Comprehensive tests:** `contracts/stream/src/test.rs` and `contracts/stream/tests/integration_suite.rs`
   - **Documentation:** `docs/` (e.g. `streaming.md`, `security.md`, `events.md`)
   - Include **Rust doc comments** on public items (NatSpec-style clarity for auditors)
   - **Validate security assumptions** (CEI ordering, token trust model, auth paths)
3. **Test and commit**
   - Run `cargo test` in `contracts/stream` (and workspace if applicable)
   - Cover **edge cases** (overflow, TTL, paused/cancelled/completed)
   - Attach **test output** summary + **security notes** in the PR

### Example commit message

```
test(stream): negative pause/resume authorization
```

## Guidelines

- Aim for **≥95%** coverage on touched Rust modules (`cargo llvm-cov` or project-standard tooling).
- **Clear documentation** (protocol semantics + operator runbooks where relevant)
- **Timeframe:** 96 hours

++++++

---
type: Feature
title: Negative tests: schedule mutations by recipient
labels: contracts, soroban, testing, security
assignees: ''
---

## Description

### Summary

Within the Fluxora streaming contract, this work tightens externally visible assurances for one focused concern. Treasury operators, recipient-facing applications, and third-party auditors must be able to reason about this area using only on-chain observables and published protocol documentation—without inferring hidden rules from how the implementation happens to be structured. The scenarios captured by this issue title must therefore have crisp success semantics, crisp failure semantics, and no silent drift between stored state, user-visible errors, and emitted events. Authorization boundaries for this slice must be explicit: who may trigger each path, and what proof they must supply. In practical terms, treat “Negative tests: schedule mutations by recipient” as the scope boundary: everything materially related to that caption should be covered by evidence (automated or documented) in this change, and anything intentionally excluded should be called out with rationale and residual risk.

**Issue caption:** Negative tests: schedule mutations by recipient

### Domain context

This repository hosts the Fluxora streaming contract on Soroban (Stellar). Streams lock funded assets in contract
storage, release entitlement over time according to schedule parameters (including cliffs), and expose explicit roles:
senders who fund and may adjust certain parameters, recipients who claim vested value, administrators who may intervene
under policy, and—where applicable—permissionless callers for maintenance reads or cleanup. Integrators (wallets,
indexers, treasury tooling) depend on coherent state transitions, emitted signals, and error behavior when inputs or
timing are invalid.

### Work to complete

1. Characterize the **intended protocol semantics** for *Negative tests: schedule mutations by recipient* in both success and failure cases, treating the **Summary** above as the authoritative scope statement.
2. Map which roles may participate, what each role must prove, and which operations must be impossible for non-authorized actors.
3. Enumerate edge cases driven by time (start, cliff, end, cancellation freeze), numeric ranges, and stream status combinations; capture expectations with tests or explicit audit exceptions.
4. Ensure externally visible behavior—persistent fields, contract errors, and emitted payloads—does not contradict documentation that integrators rely on.

### Definition of done

- An independent reader can confirm *Negative tests: schedule mutations by recipient* matches the stated intent using tests, traces, snapshots (where used), or written review notes.
- Documentation for this slice of the protocol is consistent with observable on-chain behavior after the change.
- The pull request explains verification performed, scope boundaries, and any residual assumptions or risks.

### Constraints for contributors

- Describe **outcomes**, **invariants**, and **evidence**, not a single “right” internal design unless the issue title already names a concrete subsystem.
- Prefer **observable** guarantees: state transitions, balances, authorization failures, emitted events, error classifications, and documentation that integrators rely on.
- If something cannot be tested automatically, capture the gap as **audit notes** with explicit rationale and residual risk.

## Requirements and context

- Must be **secure**, **well-tested**, and **documented**.
- Should be **gas-conscious** (Soroban budget) and **easy to audit**.

## Suggested execution

1. Fork the repo and create a branch:
   ```bash
   git checkout -b feature/negative-schedule-auth
   ```
2. Implement changes
   - **Contract / modules:** `contracts/stream/src/lib.rs`
   - **Comprehensive tests:** `contracts/stream/src/test.rs` and `contracts/stream/tests/integration_suite.rs`
   - **Documentation:** `docs/` (e.g. `streaming.md`, `security.md`, `events.md`)
   - Include **Rust doc comments** on public items (NatSpec-style clarity for auditors)
   - **Validate security assumptions** (CEI ordering, token trust model, auth paths)
3. **Test and commit**
   - Run `cargo test` in `contracts/stream` (and workspace if applicable)
   - Cover **edge cases** (overflow, TTL, paused/cancelled/completed)
   - Attach **test output** summary + **security notes** in the PR

### Example commit message

```
test(stream): negative schedule mutation auth
```

## Guidelines

- Aim for **≥95%** coverage on touched Rust modules (`cargo llvm-cov` or project-standard tooling).
- **Clear documentation** (protocol semantics + operator runbooks where relevant)
- **Timeframe:** 96 hours

++++++

---
type: Task
title: Stream ID monotonicity and uniqueness
labels: contracts, soroban, testing
assignees: ''
---

## Description

### Summary

Within the Fluxora streaming contract, this work tightens externally visible assurances for one focused concern. Treasury operators, recipient-facing applications, and third-party auditors must be able to reason about this area using only on-chain observables and published protocol documentation—without inferring hidden rules from how the implementation happens to be structured. The scenarios captured by this issue title must therefore have crisp success semantics, crisp failure semantics, and no silent drift between stored state, user-visible errors, and emitted events. Economic conservation and payout ordering assumptions must be defensible to a treasury or auditor without reference to internal layout. In practical terms, treat “Stream ID monotonicity and uniqueness” as the scope boundary: everything materially related to that caption should be covered by evidence (automated or documented) in this change, and anything intentionally excluded should be called out with rationale and residual risk.

**Issue caption:** Stream ID monotonicity and uniqueness

### Domain context

This repository hosts the Fluxora streaming contract on Soroban (Stellar). Streams lock funded assets in contract
storage, release entitlement over time according to schedule parameters (including cliffs), and expose explicit roles:
senders who fund and may adjust certain parameters, recipients who claim vested value, administrators who may intervene
under policy, and—where applicable—permissionless callers for maintenance reads or cleanup. Integrators (wallets,
indexers, treasury tooling) depend on coherent state transitions, emitted signals, and error behavior when inputs or
timing are invalid.

### Work to complete

1. Characterize the **intended protocol semantics** for *Stream ID monotonicity and uniqueness* in both success and failure cases, treating the **Summary** above as the authoritative scope statement.
2. Map which roles may participate, what each role must prove, and which operations must be impossible for non-authorized actors.
3. Enumerate edge cases driven by time (start, cliff, end, cancellation freeze), numeric ranges, and stream status combinations; capture expectations with tests or explicit audit exceptions.
4. Ensure externally visible behavior—persistent fields, contract errors, and emitted payloads—does not contradict documentation that integrators rely on.

### Definition of done

- An independent reader can confirm *Stream ID monotonicity and uniqueness* matches the stated intent using tests, traces, snapshots (where used), or written review notes.
- Documentation for this slice of the protocol is consistent with observable on-chain behavior after the change.
- The pull request explains verification performed, scope boundaries, and any residual assumptions or risks.

### Constraints for contributors

- Describe **outcomes**, **invariants**, and **evidence**, not a single “right” internal design unless the issue title already names a concrete subsystem.
- Prefer **observable** guarantees: state transitions, balances, authorization failures, emitted events, error classifications, and documentation that integrators rely on.
- If something cannot be tested automatically, capture the gap as **audit notes** with explicit rationale and residual risk.

## Requirements and context

- Must be **secure**, **well-tested**, and **documented**.
- Should be **gas-conscious** (Soroban budget) and **easy to audit**.

## Suggested execution

1. Fork the repo and create a branch:
   ```bash
   git checkout -b feature/stream-id-monotonic
   ```
2. Implement changes
   - **Contract / modules:** `contracts/stream/src/lib.rs`
   - **Comprehensive tests:** `contracts/stream/src/test.rs` and `contracts/stream/tests/integration_suite.rs`
   - **Documentation:** `docs/` (e.g. `streaming.md`, `security.md`, `events.md`)
   - Include **Rust doc comments** on public items (NatSpec-style clarity for auditors)
   - **Validate security assumptions** (CEI ordering, token trust model, auth paths)
3. **Test and commit**
   - Run `cargo test` in `contracts/stream` (and workspace if applicable)
   - Cover **edge cases** (overflow, TTL, paused/cancelled/completed)
   - Attach **test output** summary + **security notes** in the PR

### Example commit message

```
test(stream): cover stream id allocation
```

## Guidelines

- Aim for **≥95%** coverage on touched Rust modules (`cargo llvm-cov` or project-standard tooling).
- **Clear documentation** (protocol semantics + operator runbooks where relevant)
- **Timeframe:** 96 hours

++++++

---
type: Feature
title: i128 boundary streams: near-max rate/deposit scenarios
labels: contracts, soroban, safety, testing
assignees: ''
---

## Description

### Summary

Within the Fluxora streaming contract, this work tightens externally visible assurances for one focused concern. Treasury operators, recipient-facing applications, and third-party auditors must be able to reason about this area using only on-chain observables and published protocol documentation—without inferring hidden rules from how the implementation happens to be structured. The scenarios captured by this issue title must therefore have crisp success semantics, crisp failure semantics, and no silent drift between stored state, user-visible errors, and emitted events. Time boundaries (start, cliff, end, cancellation freeze) and numeric ranges must be enumerated so entitlement cannot be misread under edge timing. In practical terms, treat “i128 boundary streams: near-max rate/deposit scenarios” as the scope boundary: everything materially related to that caption should be covered by evidence (automated or documented) in this change, and anything intentionally excluded should be called out with rationale and residual risk.

**Issue caption:** i128 boundary streams: near-max rate/deposit scenarios

### Domain context

This repository hosts the Fluxora streaming contract on Soroban (Stellar). Streams lock funded assets in contract
storage, release entitlement over time according to schedule parameters (including cliffs), and expose explicit roles:
senders who fund and may adjust certain parameters, recipients who claim vested value, administrators who may intervene
under policy, and—where applicable—permissionless callers for maintenance reads or cleanup. Integrators (wallets,
indexers, treasury tooling) depend on coherent state transitions, emitted signals, and error behavior when inputs or
timing are invalid.

### Work to complete

1. Characterize the **intended protocol semantics** for *i128 boundary streams: near-max rate/deposit scenarios* in both success and failure cases, treating the **Summary** above as the authoritative scope statement.
2. Map which roles may participate, what each role must prove, and which operations must be impossible for non-authorized actors.
3. Enumerate edge cases driven by time (start, cliff, end, cancellation freeze), numeric ranges, and stream status combinations; capture expectations with tests or explicit audit exceptions.
4. Ensure externally visible behavior—persistent fields, contract errors, and emitted payloads—does not contradict documentation that integrators rely on.

### Definition of done

- An independent reader can confirm *i128 boundary streams: near-max rate/deposit scenarios* matches the stated intent using tests, traces, snapshots (where used), or written review notes.
- Documentation for this slice of the protocol is consistent with observable on-chain behavior after the change.
- The pull request explains verification performed, scope boundaries, and any residual assumptions or risks.

### Constraints for contributors

- Describe **outcomes**, **invariants**, and **evidence**, not a single “right” internal design unless the issue title already names a concrete subsystem.
- Prefer **observable** guarantees: state transitions, balances, authorization failures, emitted events, error classifications, and documentation that integrators rely on.
- If something cannot be tested automatically, capture the gap as **audit notes** with explicit rationale and residual risk.

## Requirements and context

- Must be **secure**, **well-tested**, and **documented**.
- Should be **gas-conscious** (Soroban budget) and **easy to audit**.

## Suggested execution

1. Fork the repo and create a branch:
   ```bash
   git checkout -b feature/i128-boundary-streams
   ```
2. Implement changes
   - **Contract / modules:** `contracts/stream/src/lib.rs`
   - **Comprehensive tests:** `contracts/stream/src/test.rs` and `contracts/stream/tests/integration_suite.rs`
   - **Documentation:** `docs/` (e.g. `streaming.md`, `security.md`, `events.md`)
   - Include **Rust doc comments** on public items (NatSpec-style clarity for auditors)
   - **Validate security assumptions** (CEI ordering, token trust model, auth paths)
3. **Test and commit**
   - Run `cargo test` in `contracts/stream` (and workspace if applicable)
   - Cover **edge cases** (overflow, TTL, paused/cancelled/completed)
   - Attach **test output** summary + **security notes** in the PR

### Example commit message

```
test(stream): expand i128 boundary scenarios
```

## Guidelines

- Aim for **≥95%** coverage on touched Rust modules (`cargo llvm-cov` or project-standard tooling).
- **Clear documentation** (protocol semantics + operator runbooks where relevant)
- **Timeframe:** 96 hours

++++++

---
type: Feature
title: CONTRIBUTING.md: contract test conventions
labels: contracts, soroban, docs, dx
assignees: ''
---

## Description

### Summary

Within the Fluxora streaming contract, this work tightens externally visible assurances for one focused concern. Treasury operators, recipient-facing applications, and third-party auditors must be able to reason about this area using only on-chain observables and published protocol documentation—without inferring hidden rules from how the implementation happens to be structured. The scenarios captured by this issue title must therefore have crisp success semantics, crisp failure semantics, and no silent drift between stored state, user-visible errors, and emitted events. Emitted signals and error classifications must remain coherent for indexers and wallets building on this protocol surface. In practical terms, treat “CONTRIBUTING.md: contract test conventions” as the scope boundary: everything materially related to that caption should be covered by evidence (automated or documented) in this change, and anything intentionally excluded should be called out with rationale and residual risk.

**Issue caption:** CONTRIBUTING.md: contract test conventions

### Domain context

This repository hosts the Fluxora streaming contract on Soroban (Stellar). Streams lock funded assets in contract
storage, release entitlement over time according to schedule parameters (including cliffs), and expose explicit roles:
senders who fund and may adjust certain parameters, recipients who claim vested value, administrators who may intervene
under policy, and—where applicable—permissionless callers for maintenance reads or cleanup. Integrators (wallets,
indexers, treasury tooling) depend on coherent state transitions, emitted signals, and error behavior when inputs or
timing are invalid.

### Work to complete

1. Characterize the **intended protocol semantics** for *CONTRIBUTING.md: contract test conventions* in both success and failure cases, treating the **Summary** above as the authoritative scope statement.
2. Map which roles may participate, what each role must prove, and which operations must be impossible for non-authorized actors.
3. Enumerate edge cases driven by time (start, cliff, end, cancellation freeze), numeric ranges, and stream status combinations; capture expectations with tests or explicit audit exceptions.
4. Ensure externally visible behavior—persistent fields, contract errors, and emitted payloads—does not contradict documentation that integrators rely on.

### Definition of done

- An independent reader can confirm *CONTRIBUTING.md: contract test conventions* matches the stated intent using tests, traces, snapshots (where used), or written review notes.
- Documentation for this slice of the protocol is consistent with observable on-chain behavior after the change.
- The pull request explains verification performed, scope boundaries, and any residual assumptions or risks.

### Constraints for contributors

- Describe **outcomes**, **invariants**, and **evidence**, not a single “right” internal design unless the issue title already names a concrete subsystem.
- Prefer **observable** guarantees: state transitions, balances, authorization failures, emitted events, error classifications, and documentation that integrators rely on.
- If something cannot be tested automatically, capture the gap as **audit notes** with explicit rationale and residual risk.

## Requirements and context

- Must be **secure**, **well-tested**, and **documented**.
- Should be **gas-conscious** (Soroban budget) and **easy to audit**.

## Suggested execution

1. Fork the repo and create a branch:
   ```bash
   git checkout -b feature/contributing-contract-tests
   ```
2. Implement changes
   - **Contract / modules:** `contracts/stream/src/lib.rs`
   - **Comprehensive tests:** `contracts/stream/src/test.rs` and `contracts/stream/tests/integration_suite.rs`
   - **Documentation:** `docs/` (e.g. `streaming.md`, `security.md`, `events.md`)
   - Include **Rust doc comments** on public items (NatSpec-style clarity for auditors)
   - **Validate security assumptions** (CEI ordering, token trust model, auth paths)
3. **Test and commit**
   - Run `cargo test` in `contracts/stream` (and workspace if applicable)
   - Cover **edge cases** (overflow, TTL, paused/cancelled/completed)
   - Attach **test output** summary + **security notes** in the PR

### Example commit message

```
docs: contributing guide for contract tests
```

## Guidelines

- Aim for **≥95%** coverage on touched Rust modules (`cargo llvm-cov` or project-standard tooling).
- **Clear documentation** (protocol semantics + operator runbooks where relevant)
- **Timeframe:** 96 hours

++++++

---
type: Feature
title: PR template: security + test evidence for contract changes
labels: contracts, soroban, dx
assignees: ''
---

## Description

### Summary

Within the Fluxora streaming contract, this work tightens externally visible assurances for one focused concern. Treasury operators, recipient-facing applications, and third-party auditors must be able to reason about this area using only on-chain observables and published protocol documentation—without inferring hidden rules from how the implementation happens to be structured. The scenarios captured by this issue title must therefore have crisp success semantics, crisp failure semantics, and no silent drift between stored state, user-visible errors, and emitted events. Time boundaries (start, cliff, end, cancellation freeze) and numeric ranges must be enumerated so entitlement cannot be misread under edge timing. In practical terms, treat “PR template: security + test evidence for contract changes” as the scope boundary: everything materially related to that caption should be covered by evidence (automated or documented) in this change, and anything intentionally excluded should be called out with rationale and residual risk.

**Issue caption:** PR template: security + test evidence for contract changes

### Domain context

This repository hosts the Fluxora streaming contract on Soroban (Stellar). Streams lock funded assets in contract
storage, release entitlement over time according to schedule parameters (including cliffs), and expose explicit roles:
senders who fund and may adjust certain parameters, recipients who claim vested value, administrators who may intervene
under policy, and—where applicable—permissionless callers for maintenance reads or cleanup. Integrators (wallets,
indexers, treasury tooling) depend on coherent state transitions, emitted signals, and error behavior when inputs or
timing are invalid.

### Work to complete

1. Characterize the **intended protocol semantics** for *PR template: security + test evidence for contract changes* in both success and failure cases, treating the **Summary** above as the authoritative scope statement.
2. Map which roles may participate, what each role must prove, and which operations must be impossible for non-authorized actors.
3. Enumerate edge cases driven by time (start, cliff, end, cancellation freeze), numeric ranges, and stream status combinations; capture expectations with tests or explicit audit exceptions.
4. Ensure externally visible behavior—persistent fields, contract errors, and emitted payloads—does not contradict documentation that integrators rely on.

### Definition of done

- An independent reader can confirm *PR template: security + test evidence for contract changes* matches the stated intent using tests, traces, snapshots (where used), or written review notes.
- Documentation for this slice of the protocol is consistent with observable on-chain behavior after the change.
- The pull request explains verification performed, scope boundaries, and any residual assumptions or risks.

### Constraints for contributors

- Describe **outcomes**, **invariants**, and **evidence**, not a single “right” internal design unless the issue title already names a concrete subsystem.
- Prefer **observable** guarantees: state transitions, balances, authorization failures, emitted events, error classifications, and documentation that integrators rely on.
- If something cannot be tested automatically, capture the gap as **audit notes** with explicit rationale and residual risk.

## Requirements and context

- Must be **secure**, **well-tested**, and **documented**.
- Should be **gas-conscious** (Soroban budget) and **easy to audit**.

## Suggested execution

1. Fork the repo and create a branch:
   ```bash
   git checkout -b feature/pr-template-contracts
   ```
2. Implement changes
   - **Contract / modules:** `contracts/stream/src/lib.rs`
   - **Comprehensive tests:** `contracts/stream/src/test.rs` and `contracts/stream/tests/integration_suite.rs`
   - **Documentation:** `docs/` (e.g. `streaming.md`, `security.md`, `events.md`)
   - Include **Rust doc comments** on public items (NatSpec-style clarity for auditors)
   - **Validate security assumptions** (CEI ordering, token trust model, auth paths)
3. **Test and commit**
   - Run `cargo test` in `contracts/stream` (and workspace if applicable)
   - Cover **edge cases** (overflow, TTL, paused/cancelled/completed)
   - Attach **test output** summary + **security notes** in the PR

### Example commit message

```
chore: add PR template for contract changes
```

## Guidelines

- Aim for **≥95%** coverage on touched Rust modules (`cargo llvm-cov` or project-standard tooling).
- **Clear documentation** (protocol semantics + operator runbooks where relevant)
- **Timeframe:** 24 hours

++++++

---
type: Feature
title: README alignment: replace stale 'scaffold' language
labels: contracts, soroban, docs
assignees: ''
---

## Description

### Summary

Within the Fluxora streaming contract, this work tightens externally visible assurances for one focused concern. Treasury operators, recipient-facing applications, and third-party auditors must be able to reason about this area using only on-chain observables and published protocol documentation—without inferring hidden rules from how the implementation happens to be structured. The scenarios captured by this issue title must therefore have crisp success semantics, crisp failure semantics, and no silent drift between stored state, user-visible errors, and emitted events. Time boundaries (start, cliff, end, cancellation freeze) and numeric ranges must be enumerated so entitlement cannot be misread under edge timing. In practical terms, treat “README alignment: replace stale 'scaffold' language” as the scope boundary: everything materially related to that caption should be covered by evidence (automated or documented) in this change, and anything intentionally excluded should be called out with rationale and residual risk.

**Issue caption:** README alignment: replace stale 'scaffold' language

### Domain context

This repository hosts the Fluxora streaming contract on Soroban (Stellar). Streams lock funded assets in contract
storage, release entitlement over time according to schedule parameters (including cliffs), and expose explicit roles:
senders who fund and may adjust certain parameters, recipients who claim vested value, administrators who may intervene
under policy, and—where applicable—permissionless callers for maintenance reads or cleanup. Integrators (wallets,
indexers, treasury tooling) depend on coherent state transitions, emitted signals, and error behavior when inputs or
timing are invalid.

### Work to complete

1. Characterize the **intended protocol semantics** for *README alignment: replace stale 'scaffold' language* in both success and failure cases, treating the **Summary** above as the authoritative scope statement.
2. Map which roles may participate, what each role must prove, and which operations must be impossible for non-authorized actors.
3. Enumerate edge cases driven by time (start, cliff, end, cancellation freeze), numeric ranges, and stream status combinations; capture expectations with tests or explicit audit exceptions.
4. Ensure externally visible behavior—persistent fields, contract errors, and emitted payloads—does not contradict documentation that integrators rely on.

### Definition of done

- An independent reader can confirm *README alignment: replace stale 'scaffold' language* matches the stated intent using tests, traces, snapshots (where used), or written review notes.
- Documentation for this slice of the protocol is consistent with observable on-chain behavior after the change.
- The pull request explains verification performed, scope boundaries, and any residual assumptions or risks.

### Constraints for contributors

- Describe **outcomes**, **invariants**, and **evidence**, not a single “right” internal design unless the issue title already names a concrete subsystem.
- Prefer **observable** guarantees: state transitions, balances, authorization failures, emitted events, error classifications, and documentation that integrators rely on.
- If something cannot be tested automatically, capture the gap as **audit notes** with explicit rationale and residual risk.

## Requirements and context

- Must be **secure**, **well-tested**, and **documented**.
- Should be **gas-conscious** (Soroban budget) and **easy to audit**.

## Suggested execution

1. Fork the repo and create a branch:
   ```bash
   git checkout -b feature/readme-accuracy
   ```
2. Implement changes
   - **Contract / modules:** `contracts/stream/src/lib.rs`
   - **Comprehensive tests:** `contracts/stream/src/test.rs` and `contracts/stream/tests/integration_suite.rs`
   - **Documentation:** `docs/` (e.g. `streaming.md`, `security.md`, `events.md`)
   - Include **Rust doc comments** on public items (NatSpec-style clarity for auditors)
   - **Validate security assumptions** (CEI ordering, token trust model, auth paths)
3. **Test and commit**
   - Run `cargo test` in `contracts/stream` (and workspace if applicable)
   - Cover **edge cases** (overflow, TTL, paused/cancelled/completed)
   - Attach **test output** summary + **security notes** in the PR

### Example commit message

```
docs: align README with implemented contract
```

## Guidelines

- Aim for **≥95%** coverage on touched Rust modules (`cargo llvm-cov` or project-standard tooling).
- **Clear documentation** (protocol semantics + operator runbooks where relevant)
- **Timeframe:** 96 hours

++++++

---
type: Task
title: Integration suite: cancel edge cases (fully accrued / at cliff)
labels: contracts, soroban, integration, testing
assignees: ''
---

## Description

### Summary

Within the Fluxora streaming contract, this work tightens externally visible assurances for one focused concern. Treasury operators, recipient-facing applications, and third-party auditors must be able to reason about this area using only on-chain observables and published protocol documentation—without inferring hidden rules from how the implementation happens to be structured. The scenarios captured by this issue title must therefore have crisp success semantics, crisp failure semantics, and no silent drift between stored state, user-visible errors, and emitted events. Time boundaries (start, cliff, end, cancellation freeze) and numeric ranges must be enumerated so entitlement cannot be misread under edge timing. In practical terms, treat “Integration suite: cancel edge cases (fully accrued / at cliff)” as the scope boundary: everything materially related to that caption should be covered by evidence (automated or documented) in this change, and anything intentionally excluded should be called out with rationale and residual risk.

**Issue caption:** Integration suite: cancel edge cases (fully accrued / at cliff)

### Domain context

This repository hosts the Fluxora streaming contract on Soroban (Stellar). Streams lock funded assets in contract
storage, release entitlement over time according to schedule parameters (including cliffs), and expose explicit roles:
senders who fund and may adjust certain parameters, recipients who claim vested value, administrators who may intervene
under policy, and—where applicable—permissionless callers for maintenance reads or cleanup. Integrators (wallets,
indexers, treasury tooling) depend on coherent state transitions, emitted signals, and error behavior when inputs or
timing are invalid.

### Work to complete

1. Characterize the **intended protocol semantics** for *Integration suite: cancel edge cases (fully accrued / at cliff)* in both success and failure cases, treating the **Summary** above as the authoritative scope statement.
2. Map which roles may participate, what each role must prove, and which operations must be impossible for non-authorized actors.
3. Enumerate edge cases driven by time (start, cliff, end, cancellation freeze), numeric ranges, and stream status combinations; capture expectations with tests or explicit audit exceptions.
4. Ensure externally visible behavior—persistent fields, contract errors, and emitted payloads—does not contradict documentation that integrators rely on.

### Definition of done

- An independent reader can confirm *Integration suite: cancel edge cases (fully accrued / at cliff)* matches the stated intent using tests, traces, snapshots (where used), or written review notes.
- Documentation for this slice of the protocol is consistent with observable on-chain behavior after the change.
- The pull request explains verification performed, scope boundaries, and any residual assumptions or risks.

### Constraints for contributors

- Describe **outcomes**, **invariants**, and **evidence**, not a single “right” internal design unless the issue title already names a concrete subsystem.
- Prefer **observable** guarantees: state transitions, balances, authorization failures, emitted events, error classifications, and documentation that integrators rely on.
- If something cannot be tested automatically, capture the gap as **audit notes** with explicit rationale and residual risk.

## Requirements and context

- Must be **secure**, **well-tested**, and **documented**.
- Should be **gas-conscious** (Soroban budget) and **easy to audit**.

## Suggested execution

1. Fork the repo and create a branch:
   ```bash
   git checkout -b feature/integration-cancel-edges
   ```
2. Implement changes
   - **Contract / modules:** `contracts/stream/src/lib.rs`
   - **Comprehensive tests:** `contracts/stream/src/test.rs` and `contracts/stream/tests/integration_suite.rs`
   - **Documentation:** `docs/` (e.g. `streaming.md`, `security.md`, `events.md`)
   - Include **Rust doc comments** on public items (NatSpec-style clarity for auditors)
   - **Validate security assumptions** (CEI ordering, token trust model, auth paths)
3. **Test and commit**
   - Run `cargo test` in `contracts/stream` (and workspace if applicable)
   - Cover **edge cases** (overflow, TTL, paused/cancelled/completed)
   - Attach **test output** summary + **security notes** in the PR

### Example commit message

```
test(integration): cancel edge scenarios
```

## Guidelines

- Aim for **≥95%** coverage on touched Rust modules (`cargo llvm-cov` or project-standard tooling).
- **Clear documentation** (protocol semantics + operator runbooks where relevant)
- **Timeframe:** 96 hours

++++++

---
type: Feature
title: Integration suite: pause blocks withdraw but not time semantics
labels: contracts, soroban, integration, testing
assignees: ''
---

## Description

### Summary

Within the Fluxora streaming contract, this work tightens externally visible assurances for one focused concern. Treasury operators, recipient-facing applications, and third-party auditors must be able to reason about this area using only on-chain observables and published protocol documentation—without inferring hidden rules from how the implementation happens to be structured. The scenarios captured by this issue title must therefore have crisp success semantics, crisp failure semantics, and no silent drift between stored state, user-visible errors, and emitted events. Authorization boundaries for this slice must be explicit: who may trigger each path, and what proof they must supply. In practical terms, treat “Integration suite: pause blocks withdraw but not time semantics” as the scope boundary: everything materially related to that caption should be covered by evidence (automated or documented) in this change, and anything intentionally excluded should be called out with rationale and residual risk.

**Issue caption:** Integration suite: pause blocks withdraw but not time semantics

### Domain context

This repository hosts the Fluxora streaming contract on Soroban (Stellar). Streams lock funded assets in contract
storage, release entitlement over time according to schedule parameters (including cliffs), and expose explicit roles:
senders who fund and may adjust certain parameters, recipients who claim vested value, administrators who may intervene
under policy, and—where applicable—permissionless callers for maintenance reads or cleanup. Integrators (wallets,
indexers, treasury tooling) depend on coherent state transitions, emitted signals, and error behavior when inputs or
timing are invalid.

### Work to complete

1. Characterize the **intended protocol semantics** for *Integration suite: pause blocks withdraw but not time semantics* in both success and failure cases, treating the **Summary** above as the authoritative scope statement.
2. Map which roles may participate, what each role must prove, and which operations must be impossible for non-authorized actors.
3. Enumerate edge cases driven by time (start, cliff, end, cancellation freeze), numeric ranges, and stream status combinations; capture expectations with tests or explicit audit exceptions.
4. Ensure externally visible behavior—persistent fields, contract errors, and emitted payloads—does not contradict documentation that integrators rely on.

### Definition of done

- An independent reader can confirm *Integration suite: pause blocks withdraw but not time semantics* matches the stated intent using tests, traces, snapshots (where used), or written review notes.
- Documentation for this slice of the protocol is consistent with observable on-chain behavior after the change.
- The pull request explains verification performed, scope boundaries, and any residual assumptions or risks.

### Constraints for contributors

- Describe **outcomes**, **invariants**, and **evidence**, not a single “right” internal design unless the issue title already names a concrete subsystem.
- Prefer **observable** guarantees: state transitions, balances, authorization failures, emitted events, error classifications, and documentation that integrators rely on.
- If something cannot be tested automatically, capture the gap as **audit notes** with explicit rationale and residual risk.

## Requirements and context

- Must be **secure**, **well-tested**, and **documented**.
- Should be **gas-conscious** (Soroban budget) and **easy to audit**.

## Suggested execution

1. Fork the repo and create a branch:
   ```bash
   git checkout -b feature/integration-pause-withdraw
   ```
2. Implement changes
   - **Contract / modules:** `contracts/stream/src/lib.rs`
   - **Comprehensive tests:** `contracts/stream/src/test.rs` and `contracts/stream/tests/integration_suite.rs`
   - **Documentation:** `docs/` (e.g. `streaming.md`, `security.md`, `events.md`)
   - Include **Rust doc comments** on public items (NatSpec-style clarity for auditors)
   - **Validate security assumptions** (CEI ordering, token trust model, auth paths)
3. **Test and commit**
   - Run `cargo test` in `contracts/stream` (and workspace if applicable)
   - Cover **edge cases** (overflow, TTL, paused/cancelled/completed)
   - Attach **test output** summary + **security notes** in the PR

### Example commit message

```
test(integration): pause vs withdraw interactions
```

## Guidelines

- Aim for **≥95%** coverage on touched Rust modules (`cargo llvm-cov` or project-standard tooling).
- **Clear documentation** (protocol semantics + operator runbooks where relevant)
- **Timeframe:** 96 hours

++++++

---
type: Feature
title: Storage layout docs: DataKey enum evolution policy
labels: contracts, soroban, docs, storage
assignees: ''
---

## Description

### Summary

Within the Fluxora streaming contract, this work tightens externally visible assurances for one focused concern. Treasury operators, recipient-facing applications, and third-party auditors must be able to reason about this area using only on-chain observables and published protocol documentation—without inferring hidden rules from how the implementation happens to be structured. The scenarios captured by this issue title must therefore have crisp success semantics, crisp failure semantics, and no silent drift between stored state, user-visible errors, and emitted events. Emitted signals and error classifications must remain coherent for indexers and wallets building on this protocol surface. In practical terms, treat “Storage layout docs: DataKey enum evolution policy” as the scope boundary: everything materially related to that caption should be covered by evidence (automated or documented) in this change, and anything intentionally excluded should be called out with rationale and residual risk.

**Issue caption:** Storage layout docs: DataKey enum evolution policy

### Domain context

This repository hosts the Fluxora streaming contract on Soroban (Stellar). Streams lock funded assets in contract
storage, release entitlement over time according to schedule parameters (including cliffs), and expose explicit roles:
senders who fund and may adjust certain parameters, recipients who claim vested value, administrators who may intervene
under policy, and—where applicable—permissionless callers for maintenance reads or cleanup. Integrators (wallets,
indexers, treasury tooling) depend on coherent state transitions, emitted signals, and error behavior when inputs or
timing are invalid.

### Work to complete

1. Characterize the **intended protocol semantics** for *Storage layout docs: DataKey enum evolution policy* in both success and failure cases, treating the **Summary** above as the authoritative scope statement.
2. Map which roles may participate, what each role must prove, and which operations must be impossible for non-authorized actors.
3. Enumerate edge cases driven by time (start, cliff, end, cancellation freeze), numeric ranges, and stream status combinations; capture expectations with tests or explicit audit exceptions.
4. Ensure externally visible behavior—persistent fields, contract errors, and emitted payloads—does not contradict documentation that integrators rely on.

### Definition of done

- An independent reader can confirm *Storage layout docs: DataKey enum evolution policy* matches the stated intent using tests, traces, snapshots (where used), or written review notes.
- Documentation for this slice of the protocol is consistent with observable on-chain behavior after the change.
- The pull request explains verification performed, scope boundaries, and any residual assumptions or risks.

### Constraints for contributors

- Describe **outcomes**, **invariants**, and **evidence**, not a single “right” internal design unless the issue title already names a concrete subsystem.
- Prefer **observable** guarantees: state transitions, balances, authorization failures, emitted events, error classifications, and documentation that integrators rely on.
- If something cannot be tested automatically, capture the gap as **audit notes** with explicit rationale and residual risk.

## Requirements and context

- Must be **secure**, **well-tested**, and **documented**.
- Should be **gas-conscious** (Soroban budget) and **easy to audit**.

## Suggested execution

1. Fork the repo and create a branch:
   ```bash
   git checkout -b feature/storage-layout-policy
   ```
2. Implement changes
   - **Contract / modules:** `contracts/stream/src/lib.rs`
   - **Comprehensive tests:** `contracts/stream/src/test.rs` and `contracts/stream/tests/integration_suite.rs`
   - **Documentation:** `docs/` (e.g. `streaming.md`, `security.md`, `events.md`)
   - Include **Rust doc comments** on public items (NatSpec-style clarity for auditors)
   - **Validate security assumptions** (CEI ordering, token trust model, auth paths)
3. **Test and commit**
   - Run `cargo test` in `contracts/stream` (and workspace if applicable)
   - Cover **edge cases** (overflow, TTL, paused/cancelled/completed)
   - Attach **test output** summary + **security notes** in the PR

### Example commit message

```
docs(storage): data key evolution policy
```

## Guidelines

- Aim for **≥95%** coverage on touched Rust modules (`cargo llvm-cov` or project-standard tooling).
- **Clear documentation** (protocol semantics + operator runbooks where relevant)
- **Timeframe:** 96 hours

++++++

---
type: Feature
title: Recipient index stress: many streams per recipient
labels: contracts, soroban, testing, performance
assignees: ''
---

## Description

### Summary

Within the Fluxora streaming contract, this work tightens externally visible assurances for one focused concern. Treasury operators, recipient-facing applications, and third-party auditors must be able to reason about this area using only on-chain observables and published protocol documentation—without inferring hidden rules from how the implementation happens to be structured. The scenarios captured by this issue title must therefore have crisp success semantics, crisp failure semantics, and no silent drift between stored state, user-visible errors, and emitted events. Permissionless maintenance or read paths, if involved, must have clearly documented trust impact for operators. In practical terms, treat “Recipient index stress: many streams per recipient” as the scope boundary: everything materially related to that caption should be covered by evidence (automated or documented) in this change, and anything intentionally excluded should be called out with rationale and residual risk.

**Issue caption:** Recipient index stress: many streams per recipient

### Domain context

This repository hosts the Fluxora streaming contract on Soroban (Stellar). Streams lock funded assets in contract
storage, release entitlement over time according to schedule parameters (including cliffs), and expose explicit roles:
senders who fund and may adjust certain parameters, recipients who claim vested value, administrators who may intervene
under policy, and—where applicable—permissionless callers for maintenance reads or cleanup. Integrators (wallets,
indexers, treasury tooling) depend on coherent state transitions, emitted signals, and error behavior when inputs or
timing are invalid.

### Work to complete

1. Characterize the **intended protocol semantics** for *Recipient index stress: many streams per recipient* in both success and failure cases, treating the **Summary** above as the authoritative scope statement.
2. Map which roles may participate, what each role must prove, and which operations must be impossible for non-authorized actors.
3. Enumerate edge cases driven by time (start, cliff, end, cancellation freeze), numeric ranges, and stream status combinations; capture expectations with tests or explicit audit exceptions.
4. Ensure externally visible behavior—persistent fields, contract errors, and emitted payloads—does not contradict documentation that integrators rely on.

### Definition of done

- An independent reader can confirm *Recipient index stress: many streams per recipient* matches the stated intent using tests, traces, snapshots (where used), or written review notes.
- Documentation for this slice of the protocol is consistent with observable on-chain behavior after the change.
- The pull request explains verification performed, scope boundaries, and any residual assumptions or risks.

### Constraints for contributors

- Describe **outcomes**, **invariants**, and **evidence**, not a single “right” internal design unless the issue title already names a concrete subsystem.
- Prefer **observable** guarantees: state transitions, balances, authorization failures, emitted events, error classifications, and documentation that integrators rely on.
- If something cannot be tested automatically, capture the gap as **audit notes** with explicit rationale and residual risk.

## Requirements and context

- Must be **secure**, **well-tested**, and **documented**.
- Should be **gas-conscious** (Soroban budget) and **easy to audit**.

## Suggested execution

1. Fork the repo and create a branch:
   ```bash
   git checkout -b feature/recipient-index-stress
   ```
2. Implement changes
   - **Contract / modules:** `contracts/stream/src/lib.rs`
   - **Comprehensive tests:** `contracts/stream/src/test.rs` and `contracts/stream/tests/integration_suite.rs`
   - **Documentation:** `docs/` (e.g. `streaming.md`, `security.md`, `events.md`)
   - Include **Rust doc comments** on public items (NatSpec-style clarity for auditors)
   - **Validate security assumptions** (CEI ordering, token trust model, auth paths)
3. **Test and commit**
   - Run `cargo test` in `contracts/stream` (and workspace if applicable)
   - Cover **edge cases** (overflow, TTL, paused/cancelled/completed)
   - Attach **test output** summary + **security notes** in the PR

### Example commit message

```
test(stream): stress recipient index
```

## Guidelines

- Aim for **≥95%** coverage on touched Rust modules (`cargo llvm-cov` or project-standard tooling).
- **Clear documentation** (protocol semantics + operator runbooks where relevant)
- **Timeframe:** 120 hours

++++++

---
type: Feature
title: Batch create: empty vector semantics
labels: contracts, soroban, edge-case
assignees: ''
---

## Description

### Summary

Within the Fluxora streaming contract, this work tightens externally visible assurances for one focused concern. Treasury operators, recipient-facing applications, and third-party auditors must be able to reason about this area using only on-chain observables and published protocol documentation—without inferring hidden rules from how the implementation happens to be structured. The scenarios captured by this issue title must therefore have crisp success semantics, crisp failure semantics, and no silent drift between stored state, user-visible errors, and emitted events. Economic conservation and payout ordering assumptions must be defensible to a treasury or auditor without reference to internal layout. In practical terms, treat “Batch create: empty vector semantics” as the scope boundary: everything materially related to that caption should be covered by evidence (automated or documented) in this change, and anything intentionally excluded should be called out with rationale and residual risk.

**Issue caption:** Batch create: empty vector semantics

### Domain context

This repository hosts the Fluxora streaming contract on Soroban (Stellar). Streams lock funded assets in contract
storage, release entitlement over time according to schedule parameters (including cliffs), and expose explicit roles:
senders who fund and may adjust certain parameters, recipients who claim vested value, administrators who may intervene
under policy, and—where applicable—permissionless callers for maintenance reads or cleanup. Integrators (wallets,
indexers, treasury tooling) depend on coherent state transitions, emitted signals, and error behavior when inputs or
timing are invalid.

### Work to complete

1. Characterize the **intended protocol semantics** for *Batch create: empty vector semantics* in both success and failure cases, treating the **Summary** above as the authoritative scope statement.
2. Map which roles may participate, what each role must prove, and which operations must be impossible for non-authorized actors.
3. Enumerate edge cases driven by time (start, cliff, end, cancellation freeze), numeric ranges, and stream status combinations; capture expectations with tests or explicit audit exceptions.
4. Ensure externally visible behavior—persistent fields, contract errors, and emitted payloads—does not contradict documentation that integrators rely on.

### Definition of done

- An independent reader can confirm *Batch create: empty vector semantics* matches the stated intent using tests, traces, snapshots (where used), or written review notes.
- Documentation for this slice of the protocol is consistent with observable on-chain behavior after the change.
- The pull request explains verification performed, scope boundaries, and any residual assumptions or risks.

### Constraints for contributors

- Describe **outcomes**, **invariants**, and **evidence**, not a single “right” internal design unless the issue title already names a concrete subsystem.
- Prefer **observable** guarantees: state transitions, balances, authorization failures, emitted events, error classifications, and documentation that integrators rely on.
- If something cannot be tested automatically, capture the gap as **audit notes** with explicit rationale and residual risk.

## Requirements and context

- Must be **secure**, **well-tested**, and **documented**.
- Should be **gas-conscious** (Soroban budget) and **easy to audit**.

## Suggested execution

1. Fork the repo and create a branch:
   ```bash
   git checkout -b feature/create-streams-empty
   ```
2. Implement changes
   - **Contract / modules:** `contracts/stream/src/lib.rs`
   - **Comprehensive tests:** `contracts/stream/src/test.rs` and `contracts/stream/tests/integration_suite.rs`
   - **Documentation:** `docs/` (e.g. `streaming.md`, `security.md`, `events.md`)
   - Include **Rust doc comments** on public items (NatSpec-style clarity for auditors)
   - **Validate security assumptions** (CEI ordering, token trust model, auth paths)
3. **Test and commit**
   - Run `cargo test` in `contracts/stream` (and workspace if applicable)
   - Cover **edge cases** (overflow, TTL, paused/cancelled/completed)
   - Attach **test output** summary + **security notes** in the PR

### Example commit message

```
test(stream): define empty batch create behavior
```

## Guidelines

- Aim for **≥95%** coverage on touched Rust modules (`cargo llvm-cov` or project-standard tooling).
- **Clear documentation** (protocol semantics + operator runbooks where relevant)
- **Timeframe:** 96 hours

++++++

---
type: Task
title: Batch withdraw: duplicate stream IDs in vector
labels: contracts, soroban, edge-case
assignees: ''
---

## Description

### Summary

Within the Fluxora streaming contract, this work tightens externally visible assurances for one focused concern. Treasury operators, recipient-facing applications, and third-party auditors must be able to reason about this area using only on-chain observables and published protocol documentation—without inferring hidden rules from how the implementation happens to be structured. The scenarios captured by this issue title must therefore have crisp success semantics, crisp failure semantics, and no silent drift between stored state, user-visible errors, and emitted events. Emitted signals and error classifications must remain coherent for indexers and wallets building on this protocol surface. In practical terms, treat “Batch withdraw: duplicate stream IDs in vector” as the scope boundary: everything materially related to that caption should be covered by evidence (automated or documented) in this change, and anything intentionally excluded should be called out with rationale and residual risk.

**Issue caption:** Batch withdraw: duplicate stream IDs in vector

### Domain context

This repository hosts the Fluxora streaming contract on Soroban (Stellar). Streams lock funded assets in contract
storage, release entitlement over time according to schedule parameters (including cliffs), and expose explicit roles:
senders who fund and may adjust certain parameters, recipients who claim vested value, administrators who may intervene
under policy, and—where applicable—permissionless callers for maintenance reads or cleanup. Integrators (wallets,
indexers, treasury tooling) depend on coherent state transitions, emitted signals, and error behavior when inputs or
timing are invalid.

### Work to complete

1. Characterize the **intended protocol semantics** for *Batch withdraw: duplicate stream IDs in vector* in both success and failure cases, treating the **Summary** above as the authoritative scope statement.
2. Map which roles may participate, what each role must prove, and which operations must be impossible for non-authorized actors.
3. Enumerate edge cases driven by time (start, cliff, end, cancellation freeze), numeric ranges, and stream status combinations; capture expectations with tests or explicit audit exceptions.
4. Ensure externally visible behavior—persistent fields, contract errors, and emitted payloads—does not contradict documentation that integrators rely on.

### Definition of done

- An independent reader can confirm *Batch withdraw: duplicate stream IDs in vector* matches the stated intent using tests, traces, snapshots (where used), or written review notes.
- Documentation for this slice of the protocol is consistent with observable on-chain behavior after the change.
- The pull request explains verification performed, scope boundaries, and any residual assumptions or risks.

### Constraints for contributors

- Describe **outcomes**, **invariants**, and **evidence**, not a single “right” internal design unless the issue title already names a concrete subsystem.
- Prefer **observable** guarantees: state transitions, balances, authorization failures, emitted events, error classifications, and documentation that integrators rely on.
- If something cannot be tested automatically, capture the gap as **audit notes** with explicit rationale and residual risk.

## Requirements and context

- Must be **secure**, **well-tested**, and **documented**.
- Should be **gas-conscious** (Soroban budget) and **easy to audit**.

## Suggested execution

1. Fork the repo and create a branch:
   ```bash
   git checkout -b feature/batch-withdraw-duplicates
   ```
2. Implement changes
   - **Contract / modules:** `contracts/stream/src/lib.rs`
   - **Comprehensive tests:** `contracts/stream/src/test.rs` and `contracts/stream/tests/integration_suite.rs`
   - **Documentation:** `docs/` (e.g. `streaming.md`, `security.md`, `events.md`)
   - Include **Rust doc comments** on public items (NatSpec-style clarity for auditors)
   - **Validate security assumptions** (CEI ordering, token trust model, auth paths)
3. **Test and commit**
   - Run `cargo test` in `contracts/stream` (and workspace if applicable)
   - Cover **edge cases** (overflow, TTL, paused/cancelled/completed)
   - Attach **test output** summary + **security notes** in the PR

### Example commit message

```
test(stream): batch withdraw duplicate ids
```

## Guidelines

- Aim for **≥95%** coverage on touched Rust modules (`cargo llvm-cov` or project-standard tooling).
- **Clear documentation** (protocol semantics + operator runbooks where relevant)
- **Timeframe:** 96 hours

++++++

---
type: Feature
title: Admin event parity: ensure admin ops emit same downstream topics
labels: contracts, soroban, indexing
assignees: ''
---

## Description

### Summary

Within the Fluxora streaming contract, this work tightens externally visible assurances for one focused concern. Treasury operators, recipient-facing applications, and third-party auditors must be able to reason about this area using only on-chain observables and published protocol documentation—without inferring hidden rules from how the implementation happens to be structured. The scenarios captured by this issue title must therefore have crisp success semantics, crisp failure semantics, and no silent drift between stored state, user-visible errors, and emitted events. Time boundaries (start, cliff, end, cancellation freeze) and numeric ranges must be enumerated so entitlement cannot be misread under edge timing. In practical terms, treat “Admin event parity: ensure admin ops emit same downstream topics” as the scope boundary: everything materially related to that caption should be covered by evidence (automated or documented) in this change, and anything intentionally excluded should be called out with rationale and residual risk.

**Issue caption:** Admin event parity: ensure admin ops emit same downstream topics

### Domain context

This repository hosts the Fluxora streaming contract on Soroban (Stellar). Streams lock funded assets in contract
storage, release entitlement over time according to schedule parameters (including cliffs), and expose explicit roles:
senders who fund and may adjust certain parameters, recipients who claim vested value, administrators who may intervene
under policy, and—where applicable—permissionless callers for maintenance reads or cleanup. Integrators (wallets,
indexers, treasury tooling) depend on coherent state transitions, emitted signals, and error behavior when inputs or
timing are invalid.

### Work to complete

1. Characterize the **intended protocol semantics** for *Admin event parity: ensure admin ops emit same downstream topics* in both success and failure cases, treating the **Summary** above as the authoritative scope statement.
2. Map which roles may participate, what each role must prove, and which operations must be impossible for non-authorized actors.
3. Enumerate edge cases driven by time (start, cliff, end, cancellation freeze), numeric ranges, and stream status combinations; capture expectations with tests or explicit audit exceptions.
4. Ensure externally visible behavior—persistent fields, contract errors, and emitted payloads—does not contradict documentation that integrators rely on.

### Definition of done

- An independent reader can confirm *Admin event parity: ensure admin ops emit same downstream topics* matches the stated intent using tests, traces, snapshots (where used), or written review notes.
- Documentation for this slice of the protocol is consistent with observable on-chain behavior after the change.
- The pull request explains verification performed, scope boundaries, and any residual assumptions or risks.

### Constraints for contributors

- Describe **outcomes**, **invariants**, and **evidence**, not a single “right” internal design unless the issue title already names a concrete subsystem.
- Prefer **observable** guarantees: state transitions, balances, authorization failures, emitted events, error classifications, and documentation that integrators rely on.
- If something cannot be tested automatically, capture the gap as **audit notes** with explicit rationale and residual risk.

## Requirements and context

- Must be **secure**, **well-tested**, and **documented**.
- Should be **gas-conscious** (Soroban budget) and **easy to audit**.

## Suggested execution

1. Fork the repo and create a branch:
   ```bash
   git checkout -b feature/admin-event-parity
   ```
2. Implement changes
   - **Contract / modules:** `contracts/stream/src/lib.rs`
   - **Comprehensive tests:** `contracts/stream/src/test.rs` and `contracts/stream/tests/integration_suite.rs`
   - **Documentation:** `docs/` (e.g. `streaming.md`, `security.md`, `events.md`)
   - Include **Rust doc comments** on public items (NatSpec-style clarity for auditors)
   - **Validate security assumptions** (CEI ordering, token trust model, auth paths)
3. **Test and commit**
   - Run `cargo test` in `contracts/stream` (and workspace if applicable)
   - Cover **edge cases** (overflow, TTL, paused/cancelled/completed)
   - Attach **test output** summary + **security notes** in the PR

### Example commit message

```
test(stream): admin vs sender event parity
```

## Guidelines

- Aim for **≥95%** coverage on touched Rust modules (`cargo llvm-cov` or project-standard tooling).
- **Clear documentation** (protocol semantics + operator runbooks where relevant)
- **Timeframe:** 96 hours

++++++

---
type: Feature
title: Upgrade strategy: CONTRACT_VERSION policy + migration notes
labels: contracts, soroban, ops, docs
assignees: ''
---

## Description

### Summary

Within the Fluxora streaming contract, this work tightens externally visible assurances for one focused concern. Treasury operators, recipient-facing applications, and third-party auditors must be able to reason about this area using only on-chain observables and published protocol documentation—without inferring hidden rules from how the implementation happens to be structured. The scenarios captured by this issue title must therefore have crisp success semantics, crisp failure semantics, and no silent drift between stored state, user-visible errors, and emitted events. Time boundaries (start, cliff, end, cancellation freeze) and numeric ranges must be enumerated so entitlement cannot be misread under edge timing. In practical terms, treat “Upgrade strategy: CONTRACT_VERSION policy + migration notes” as the scope boundary: everything materially related to that caption should be covered by evidence (automated or documented) in this change, and anything intentionally excluded should be called out with rationale and residual risk.

**Issue caption:** Upgrade strategy: CONTRACT_VERSION policy + migration notes

### Domain context

This repository hosts the Fluxora streaming contract on Soroban (Stellar). Streams lock funded assets in contract
storage, release entitlement over time according to schedule parameters (including cliffs), and expose explicit roles:
senders who fund and may adjust certain parameters, recipients who claim vested value, administrators who may intervene
under policy, and—where applicable—permissionless callers for maintenance reads or cleanup. Integrators (wallets,
indexers, treasury tooling) depend on coherent state transitions, emitted signals, and error behavior when inputs or
timing are invalid.

### Work to complete

1. Characterize the **intended protocol semantics** for *Upgrade strategy: CONTRACT_VERSION policy + migration notes* in both success and failure cases, treating the **Summary** above as the authoritative scope statement.
2. Map which roles may participate, what each role must prove, and which operations must be impossible for non-authorized actors.
3. Enumerate edge cases driven by time (start, cliff, end, cancellation freeze), numeric ranges, and stream status combinations; capture expectations with tests or explicit audit exceptions.
4. Ensure externally visible behavior—persistent fields, contract errors, and emitted payloads—does not contradict documentation that integrators rely on.

### Definition of done

- An independent reader can confirm *Upgrade strategy: CONTRACT_VERSION policy + migration notes* matches the stated intent using tests, traces, snapshots (where used), or written review notes.
- Documentation for this slice of the protocol is consistent with observable on-chain behavior after the change.
- The pull request explains verification performed, scope boundaries, and any residual assumptions or risks.

### Constraints for contributors

- Describe **outcomes**, **invariants**, and **evidence**, not a single “right” internal design unless the issue title already names a concrete subsystem.
- Prefer **observable** guarantees: state transitions, balances, authorization failures, emitted events, error classifications, and documentation that integrators rely on.
- If something cannot be tested automatically, capture the gap as **audit notes** with explicit rationale and residual risk.

## Requirements and context

- Must be **secure**, **well-tested**, and **documented**.
- Should be **gas-conscious** (Soroban budget) and **easy to audit**.

## Suggested execution

1. Fork the repo and create a branch:
   ```bash
   git checkout -b feature/versioning-strategy
   ```
2. Implement changes
   - **Contract / modules:** `contracts/stream/src/lib.rs`
   - **Comprehensive tests:** `contracts/stream/src/test.rs` and `contracts/stream/tests/integration_suite.rs`
   - **Documentation:** `docs/` (e.g. `streaming.md`, `security.md`, `events.md`)
   - Include **Rust doc comments** on public items (NatSpec-style clarity for auditors)
   - **Validate security assumptions** (CEI ordering, token trust model, auth paths)
3. **Test and commit**
   - Run `cargo test` in `contracts/stream` (and workspace if applicable)
   - Cover **edge cases** (overflow, TTL, paused/cancelled/completed)
   - Attach **test output** summary + **security notes** in the PR

### Example commit message

```
docs: contract upgrade and versioning strategy
```

## Guidelines

- Aim for **≥95%** coverage on touched Rust modules (`cargo llvm-cov` or project-standard tooling).
- **Clear documentation** (protocol semantics + operator runbooks where relevant)
- **Timeframe:** 96 hours

++++++

---
type: Feature
title: CI: run fmt + clippy on contracts workspace
labels: contracts, soroban, ci
assignees: ''
---

## Description

### Summary

Within the Fluxora streaming contract, this work tightens externally visible assurances for one focused concern. Treasury operators, recipient-facing applications, and third-party auditors must be able to reason about this area using only on-chain observables and published protocol documentation—without inferring hidden rules from how the implementation happens to be structured. The scenarios captured by this issue title must therefore have crisp success semantics, crisp failure semantics, and no silent drift between stored state, user-visible errors, and emitted events. Time boundaries (start, cliff, end, cancellation freeze) and numeric ranges must be enumerated so entitlement cannot be misread under edge timing. In practical terms, treat “CI: run fmt + clippy on contracts workspace” as the scope boundary: everything materially related to that caption should be covered by evidence (automated or documented) in this change, and anything intentionally excluded should be called out with rationale and residual risk.

**Issue caption:** CI: run fmt + clippy on contracts workspace

### Domain context

This repository hosts the Fluxora streaming contract on Soroban (Stellar). Streams lock funded assets in contract
storage, release entitlement over time according to schedule parameters (including cliffs), and expose explicit roles:
senders who fund and may adjust certain parameters, recipients who claim vested value, administrators who may intervene
under policy, and—where applicable—permissionless callers for maintenance reads or cleanup. Integrators (wallets,
indexers, treasury tooling) depend on coherent state transitions, emitted signals, and error behavior when inputs or
timing are invalid.

### Work to complete

1. Characterize the **intended protocol semantics** for *CI: run fmt + clippy on contracts workspace* in both success and failure cases, treating the **Summary** above as the authoritative scope statement.
2. Map which roles may participate, what each role must prove, and which operations must be impossible for non-authorized actors.
3. Enumerate edge cases driven by time (start, cliff, end, cancellation freeze), numeric ranges, and stream status combinations; capture expectations with tests or explicit audit exceptions.
4. Ensure externally visible behavior—persistent fields, contract errors, and emitted payloads—does not contradict documentation that integrators rely on.

### Definition of done

- An independent reader can confirm *CI: run fmt + clippy on contracts workspace* matches the stated intent using tests, traces, snapshots (where used), or written review notes.
- Documentation for this slice of the protocol is consistent with observable on-chain behavior after the change.
- The pull request explains verification performed, scope boundaries, and any residual assumptions or risks.

### Constraints for contributors

- Describe **outcomes**, **invariants**, and **evidence**, not a single “right” internal design unless the issue title already names a concrete subsystem.
- Prefer **observable** guarantees: state transitions, balances, authorization failures, emitted events, error classifications, and documentation that integrators rely on.
- If something cannot be tested automatically, capture the gap as **audit notes** with explicit rationale and residual risk.

## Requirements and context

- Must be **secure**, **well-tested**, and **documented**.
- Should be **gas-conscious** (Soroban budget) and **easy to audit**.

## Suggested execution

1. Fork the repo and create a branch:
   ```bash
   git checkout -b feature/ci-fmt-clippy
   ```
2. Implement changes
   - **Contract / modules:** `contracts/stream/src/lib.rs`
   - **Comprehensive tests:** `contracts/stream/src/test.rs` and `contracts/stream/tests/integration_suite.rs`
   - **Documentation:** `docs/` (e.g. `streaming.md`, `security.md`, `events.md`)
   - Include **Rust doc comments** on public items (NatSpec-style clarity for auditors)
   - **Validate security assumptions** (CEI ordering, token trust model, auth paths)
3. **Test and commit**
   - Run `cargo test` in `contracts/stream` (and workspace if applicable)
   - Cover **edge cases** (overflow, TTL, paused/cancelled/completed)
   - Attach **test output** summary + **security notes** in the PR

### Example commit message

```
ci: enforce fmt and clippy for stream crate
```

## Guidelines

- Aim for **≥95%** coverage on touched Rust modules (`cargo llvm-cov` or project-standard tooling).
- **Clear documentation** (protocol semantics + operator runbooks where relevant)
- **Timeframe:** 96 hours

++++++

---
type: Feature
title: Ledger timestamp manipulation assumptions in tests
labels: contracts, soroban, testing
assignees: ''
---

## Description

### Summary

Within the Fluxora streaming contract, this work tightens externally visible assurances for one focused concern. Treasury operators, recipient-facing applications, and third-party auditors must be able to reason about this area using only on-chain observables and published protocol documentation—without inferring hidden rules from how the implementation happens to be structured. The scenarios captured by this issue title must therefore have crisp success semantics, crisp failure semantics, and no silent drift between stored state, user-visible errors, and emitted events. Time boundaries (start, cliff, end, cancellation freeze) and numeric ranges must be enumerated so entitlement cannot be misread under edge timing. In practical terms, treat “Ledger timestamp manipulation assumptions in tests” as the scope boundary: everything materially related to that caption should be covered by evidence (automated or documented) in this change, and anything intentionally excluded should be called out with rationale and residual risk.

**Issue caption:** Ledger timestamp manipulation assumptions in tests

### Domain context

This repository hosts the Fluxora streaming contract on Soroban (Stellar). Streams lock funded assets in contract
storage, release entitlement over time according to schedule parameters (including cliffs), and expose explicit roles:
senders who fund and may adjust certain parameters, recipients who claim vested value, administrators who may intervene
under policy, and—where applicable—permissionless callers for maintenance reads or cleanup. Integrators (wallets,
indexers, treasury tooling) depend on coherent state transitions, emitted signals, and error behavior when inputs or
timing are invalid.

### Work to complete

1. Characterize the **intended protocol semantics** for *Ledger timestamp manipulation assumptions in tests* in both success and failure cases, treating the **Summary** above as the authoritative scope statement.
2. Map which roles may participate, what each role must prove, and which operations must be impossible for non-authorized actors.
3. Enumerate edge cases driven by time (start, cliff, end, cancellation freeze), numeric ranges, and stream status combinations; capture expectations with tests or explicit audit exceptions.
4. Ensure externally visible behavior—persistent fields, contract errors, and emitted payloads—does not contradict documentation that integrators rely on.

### Definition of done

- An independent reader can confirm *Ledger timestamp manipulation assumptions in tests* matches the stated intent using tests, traces, snapshots (where used), or written review notes.
- Documentation for this slice of the protocol is consistent with observable on-chain behavior after the change.
- The pull request explains verification performed, scope boundaries, and any residual assumptions or risks.

### Constraints for contributors

- Describe **outcomes**, **invariants**, and **evidence**, not a single “right” internal design unless the issue title already names a concrete subsystem.
- Prefer **observable** guarantees: state transitions, balances, authorization failures, emitted events, error classifications, and documentation that integrators rely on.
- If something cannot be tested automatically, capture the gap as **audit notes** with explicit rationale and residual risk.

## Requirements and context

- Must be **secure**, **well-tested**, and **documented**.
- Should be **gas-conscious** (Soroban budget) and **easy to audit**.

## Suggested execution

1. Fork the repo and create a branch:
   ```bash
   git checkout -b feature/test-time-assumptions
   ```
2. Implement changes
   - **Contract / modules:** `contracts/stream/src/lib.rs`
   - **Comprehensive tests:** `contracts/stream/src/test.rs` and `contracts/stream/tests/integration_suite.rs`
   - **Documentation:** `docs/` (e.g. `streaming.md`, `security.md`, `events.md`)
   - Include **Rust doc comments** on public items (NatSpec-style clarity for auditors)
   - **Validate security assumptions** (CEI ordering, token trust model, auth paths)
3. **Test and commit**
   - Run `cargo test` in `contracts/stream` (and workspace if applicable)
   - Cover **edge cases** (overflow, TTL, paused/cancelled/completed)
   - Attach **test output** summary + **security notes** in the PR

### Example commit message

```
docs(testing): ledger timestamp assumptions
```

## Guidelines

- Aim for **≥95%** coverage on touched Rust modules (`cargo llvm-cov` or project-standard tooling).
- **Clear documentation** (protocol semantics + operator runbooks where relevant)
- **Timeframe:** 96 hours

++++++

---
type: Task
title: Rate update interaction with partial withdrawals
labels: contracts, soroban, testing
assignees: ''
---

## Description

### Summary

Within the Fluxora streaming contract, this work tightens externally visible assurances for one focused concern. Treasury operators, recipient-facing applications, and third-party auditors must be able to reason about this area using only on-chain observables and published protocol documentation—without inferring hidden rules from how the implementation happens to be structured. The scenarios captured by this issue title must therefore have crisp success semantics, crisp failure semantics, and no silent drift between stored state, user-visible errors, and emitted events. Permissionless maintenance or read paths, if involved, must have clearly documented trust impact for operators. In practical terms, treat “Rate update interaction with partial withdrawals” as the scope boundary: everything materially related to that caption should be covered by evidence (automated or documented) in this change, and anything intentionally excluded should be called out with rationale and residual risk.

**Issue caption:** Rate update interaction with partial withdrawals

### Domain context

This repository hosts the Fluxora streaming contract on Soroban (Stellar). Streams lock funded assets in contract
storage, release entitlement over time according to schedule parameters (including cliffs), and expose explicit roles:
senders who fund and may adjust certain parameters, recipients who claim vested value, administrators who may intervene
under policy, and—where applicable—permissionless callers for maintenance reads or cleanup. Integrators (wallets,
indexers, treasury tooling) depend on coherent state transitions, emitted signals, and error behavior when inputs or
timing are invalid.

### Work to complete

1. Characterize the **intended protocol semantics** for *Rate update interaction with partial withdrawals* in both success and failure cases, treating the **Summary** above as the authoritative scope statement.
2. Map which roles may participate, what each role must prove, and which operations must be impossible for non-authorized actors.
3. Enumerate edge cases driven by time (start, cliff, end, cancellation freeze), numeric ranges, and stream status combinations; capture expectations with tests or explicit audit exceptions.
4. Ensure externally visible behavior—persistent fields, contract errors, and emitted payloads—does not contradict documentation that integrators rely on.

### Definition of done

- An independent reader can confirm *Rate update interaction with partial withdrawals* matches the stated intent using tests, traces, snapshots (where used), or written review notes.
- Documentation for this slice of the protocol is consistent with observable on-chain behavior after the change.
- The pull request explains verification performed, scope boundaries, and any residual assumptions or risks.

### Constraints for contributors

- Describe **outcomes**, **invariants**, and **evidence**, not a single “right” internal design unless the issue title already names a concrete subsystem.
- Prefer **observable** guarantees: state transitions, balances, authorization failures, emitted events, error classifications, and documentation that integrators rely on.
- If something cannot be tested automatically, capture the gap as **audit notes** with explicit rationale and residual risk.

## Requirements and context

- Must be **secure**, **well-tested**, and **documented**.
- Should be **gas-conscious** (Soroban budget) and **easy to audit**.

## Suggested execution

1. Fork the repo and create a branch:
   ```bash
   git checkout -b feature/rate-update-after-partial
   ```
2. Implement changes
   - **Contract / modules:** `contracts/stream/src/lib.rs`
   - **Comprehensive tests:** `contracts/stream/src/test.rs` and `contracts/stream/tests/integration_suite.rs`
   - **Documentation:** `docs/` (e.g. `streaming.md`, `security.md`, `events.md`)
   - Include **Rust doc comments** on public items (NatSpec-style clarity for auditors)
   - **Validate security assumptions** (CEI ordering, token trust model, auth paths)
3. **Test and commit**
   - Run `cargo test` in `contracts/stream` (and workspace if applicable)
   - Cover **edge cases** (overflow, TTL, paused/cancelled/completed)
   - Attach **test output** summary + **security notes** in the PR

### Example commit message

```
test(stream): rate update with partial withdraw
```

## Guidelines

- Aim for **≥95%** coverage on touched Rust modules (`cargo llvm-cov` or project-standard tooling).
- **Clear documentation** (protocol semantics + operator runbooks where relevant)
- **Timeframe:** 96 hours

++++++

---
type: Feature
title: Top-up interaction with near-complete streams
labels: contracts, soroban, testing
assignees: ''
---

## Description

### Summary

Within the Fluxora streaming contract, this work tightens externally visible assurances for one focused concern. Treasury operators, recipient-facing applications, and third-party auditors must be able to reason about this area using only on-chain observables and published protocol documentation—without inferring hidden rules from how the implementation happens to be structured. The scenarios captured by this issue title must therefore have crisp success semantics, crisp failure semantics, and no silent drift between stored state, user-visible errors, and emitted events. Permissionless maintenance or read paths, if involved, must have clearly documented trust impact for operators. In practical terms, treat “Top-up interaction with near-complete streams” as the scope boundary: everything materially related to that caption should be covered by evidence (automated or documented) in this change, and anything intentionally excluded should be called out with rationale and residual risk.

**Issue caption:** Top-up interaction with near-complete streams

### Domain context

This repository hosts the Fluxora streaming contract on Soroban (Stellar). Streams lock funded assets in contract
storage, release entitlement over time according to schedule parameters (including cliffs), and expose explicit roles:
senders who fund and may adjust certain parameters, recipients who claim vested value, administrators who may intervene
under policy, and—where applicable—permissionless callers for maintenance reads or cleanup. Integrators (wallets,
indexers, treasury tooling) depend on coherent state transitions, emitted signals, and error behavior when inputs or
timing are invalid.

### Work to complete

1. Characterize the **intended protocol semantics** for *Top-up interaction with near-complete streams* in both success and failure cases, treating the **Summary** above as the authoritative scope statement.
2. Map which roles may participate, what each role must prove, and which operations must be impossible for non-authorized actors.
3. Enumerate edge cases driven by time (start, cliff, end, cancellation freeze), numeric ranges, and stream status combinations; capture expectations with tests or explicit audit exceptions.
4. Ensure externally visible behavior—persistent fields, contract errors, and emitted payloads—does not contradict documentation that integrators rely on.

### Definition of done

- An independent reader can confirm *Top-up interaction with near-complete streams* matches the stated intent using tests, traces, snapshots (where used), or written review notes.
- Documentation for this slice of the protocol is consistent with observable on-chain behavior after the change.
- The pull request explains verification performed, scope boundaries, and any residual assumptions or risks.

### Constraints for contributors

- Describe **outcomes**, **invariants**, and **evidence**, not a single “right” internal design unless the issue title already names a concrete subsystem.
- Prefer **observable** guarantees: state transitions, balances, authorization failures, emitted events, error classifications, and documentation that integrators rely on.
- If something cannot be tested automatically, capture the gap as **audit notes** with explicit rationale and residual risk.

## Requirements and context

- Must be **secure**, **well-tested**, and **documented**.
- Should be **gas-conscious** (Soroban budget) and **easy to audit**.

## Suggested execution

1. Fork the repo and create a branch:
   ```bash
   git checkout -b feature/top-up-near-complete
   ```
2. Implement changes
   - **Contract / modules:** `contracts/stream/src/lib.rs`
   - **Comprehensive tests:** `contracts/stream/src/test.rs` and `contracts/stream/tests/integration_suite.rs`
   - **Documentation:** `docs/` (e.g. `streaming.md`, `security.md`, `events.md`)
   - Include **Rust doc comments** on public items (NatSpec-style clarity for auditors)
   - **Validate security assumptions** (CEI ordering, token trust model, auth paths)
3. **Test and commit**
   - Run `cargo test` in `contracts/stream` (and workspace if applicable)
   - Cover **edge cases** (overflow, TTL, paused/cancelled/completed)
   - Attach **test output** summary + **security notes** in the PR

### Example commit message

```
test(stream): top up near completion
```

## Guidelines

- Aim for **≥95%** coverage on touched Rust modules (`cargo llvm-cov` or project-standard tooling).
- **Clear documentation** (protocol semantics + operator runbooks where relevant)
- **Timeframe:** 96 hours

++++++

---
type: Feature
title: Global pause interaction with withdrawals (should not block)
labels: contracts, soroban, governance, testing
assignees: ''
---

## Description

### Summary

Within the Fluxora streaming contract, this work tightens externally visible assurances for one focused concern. Treasury operators, recipient-facing applications, and third-party auditors must be able to reason about this area using only on-chain observables and published protocol documentation—without inferring hidden rules from how the implementation happens to be structured. The scenarios captured by this issue title must therefore have crisp success semantics, crisp failure semantics, and no silent drift between stored state, user-visible errors, and emitted events. Authorization boundaries for this slice must be explicit: who may trigger each path, and what proof they must supply. In practical terms, treat “Global pause interaction with withdrawals (should not block)” as the scope boundary: everything materially related to that caption should be covered by evidence (automated or documented) in this change, and anything intentionally excluded should be called out with rationale and residual risk.

**Issue caption:** Global pause interaction with withdrawals (should not block)

### Domain context

This repository hosts the Fluxora streaming contract on Soroban (Stellar). Streams lock funded assets in contract
storage, release entitlement over time according to schedule parameters (including cliffs), and expose explicit roles:
senders who fund and may adjust certain parameters, recipients who claim vested value, administrators who may intervene
under policy, and—where applicable—permissionless callers for maintenance reads or cleanup. Integrators (wallets,
indexers, treasury tooling) depend on coherent state transitions, emitted signals, and error behavior when inputs or
timing are invalid.

### Work to complete

1. Characterize the **intended protocol semantics** for *Global pause interaction with withdrawals (should not block)* in both success and failure cases, treating the **Summary** above as the authoritative scope statement.
2. Map which roles may participate, what each role must prove, and which operations must be impossible for non-authorized actors.
3. Enumerate edge cases driven by time (start, cliff, end, cancellation freeze), numeric ranges, and stream status combinations; capture expectations with tests or explicit audit exceptions.
4. Ensure externally visible behavior—persistent fields, contract errors, and emitted payloads—does not contradict documentation that integrators rely on.

### Definition of done

- An independent reader can confirm *Global pause interaction with withdrawals (should not block)* matches the stated intent using tests, traces, snapshots (where used), or written review notes.
- Documentation for this slice of the protocol is consistent with observable on-chain behavior after the change.
- The pull request explains verification performed, scope boundaries, and any residual assumptions or risks.

### Constraints for contributors

- Describe **outcomes**, **invariants**, and **evidence**, not a single “right” internal design unless the issue title already names a concrete subsystem.
- Prefer **observable** guarantees: state transitions, balances, authorization failures, emitted events, error classifications, and documentation that integrators rely on.
- If something cannot be tested automatically, capture the gap as **audit notes** with explicit rationale and residual risk.

## Requirements and context

- Must be **secure**, **well-tested**, and **documented**.
- Should be **gas-conscious** (Soroban budget) and **easy to audit**.

## Suggested execution

1. Fork the repo and create a branch:
   ```bash
   git checkout -b feature/global-pause-vs-withdraw
   ```
2. Implement changes
   - **Contract / modules:** `contracts/stream/src/lib.rs`
   - **Comprehensive tests:** `contracts/stream/src/test.rs` and `contracts/stream/tests/integration_suite.rs`
   - **Documentation:** `docs/` (e.g. `streaming.md`, `security.md`, `events.md`)
   - Include **Rust doc comments** on public items (NatSpec-style clarity for auditors)
   - **Validate security assumptions** (CEI ordering, token trust model, auth paths)
3. **Test and commit**
   - Run `cargo test` in `contracts/stream` (and workspace if applicable)
   - Cover **edge cases** (overflow, TTL, paused/cancelled/completed)
   - Attach **test output** summary + **security notes** in the PR

### Example commit message

```
test(stream): global pause vs withdrawals
```

## Guidelines

- Aim for **≥95%** coverage on touched Rust modules (`cargo llvm-cov` or project-standard tooling).
- **Clear documentation** (protocol semantics + operator runbooks where relevant)
- **Timeframe:** 96 hours
