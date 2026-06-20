# Governance Contract

## Purpose

The `FluxoraGovernance` contract (`contracts/governance/src/lib.rs`) implements a
configurable-threshold proposal / approve / execute governance pattern with a timelock.
It decouples operational signing keys from protocol-parameter authority: no single key can
change factory parameters immediately; a threshold of co-signers must approve and a mandatory
waiting period must elapse before the change is considered executable.

## Threshold model

The approval `threshold` is set at `init` time and stored in instance storage. It represents
the minimum number of co-signer approvals required before a proposal can be executed. The
invariant `1 <= threshold <= signers.len()` is enforced at:

- `init`: the initial threshold must be between 1 and the initial signer count.
- `remove_signer`: removal is rejected with `QuorumWouldBreak` if it would leave fewer
  signers than the current threshold.
- `add_signer`: the threshold is unchanged, so adding signers can never violate the invariant.

When quorum is first reached, the current threshold is snapshotted alongside the timestamp in
a `QuorumInfo` record. At execution time the proposal is judged against this snapshot, making
in-flight proposals immune to later threshold changes by the admin.

## Constants

| Constant | Value | Meaning |
|---|---:|---|
| `GOVERNANCE_TIMELOCK_SECONDS` | 172,800 (48 h) | Seconds to wait after quorum before executing |
| `MAX_PROPOSAL_AGE_SECONDS` | 2,592,000 (30 d) | Maximum proposal age before approval and execution are rejected |
| `MAX_SIGNERS` | 20 | Maximum co-signers registered at once |
| `MAX_CALLDATA_BYTES` | 4,096 | Maximum byte length for the `calldata` field |

## Roles

### Admin

Set at `init` time. The admin can:

- Add or remove co-signers with `add_signer` and `remove_signer`.
- Rotate the admin address with `set_admin`.

The admin alone cannot execute proposals. If the admin should count toward quorum, that
address must also be registered as a co-signer and call `approve`.

### Co-signers

A fixed set of addresses registered by the admin. Co-signers can:

- Submit proposals with `propose`.
- Approve existing proposals with `approve`.

Each co-signer address may appear only once. `init` and `add_signer` reject duplicate
addresses with `DuplicateSigner`, so quorum calculations are based on unique keys. The
proposer is not automatically counted as an approver; they must submit a separate `approve`
call if their signature should count toward quorum.

## Proposal lifecycle

```text
propose()      -> Proposed with zero approvals
approve()      -> Approved below threshold
approve()      -> Quorum reached; timelock starts
wait timelock  -> Executable
execute()      -> Executed, terminal

cancel_proposal()              -> Cancelled, terminal
after MAX_PROPOSAL_AGE_SECONDS -> Expired, terminal
```

### State semantics

- `Proposed`: `propose` stores a `Proposal` with zero approvals, `executed = false`, and
  `cancelled = false`.
- `Approved`: at least one signer has approved, but the approval count is still below the
  effective threshold.
- `QuorumReached`: the approval that makes `approval_count == threshold` stores
  `DataKey::QuorumReachedAt(proposal_id)` as `QuorumInfo { reached_at, threshold }` and
  emits `QuorumReached`.
- `Executable`: this is a derived client/indexer state, not a stored enum. A proposal is
  executable when the ledger timestamp is greater than or equal to
  `quorum_reached_at + GOVERNANCE_TIMELOCK_SECONDS`.
- `Executed`: `execute` sets `proposal.executed = true` before emitting
  `ProposalExecuted`.
- `Cancelled`: `cancel_proposal` sets `proposal.cancelled = true` and emits
  `ProposalCancelled`.
- `Expired`: this is a derived terminal state when
  `ledger.timestamp() > proposal.created_at + MAX_PROPOSAL_AGE_SECONDS`.

Cancelled, expired, and executed proposals cannot receive approvals or be executed again.
Additional approvals above quorum do not rewrite `QuorumInfo` and do not restart the
timelock.

## Entrypoints

### `init(admin, signers, threshold)`

Initializes the contract. It can only be called once.

- Fails with `AlreadyInitialized` if `Admin` already exists.
- Fails with `TooManySigners` if `signers.len() > MAX_SIGNERS`.
- Fails with `DuplicateSigner` if a signer appears more than once.
- Fails with `InvalidThreshold` unless `1 <= threshold <= signers.len()`.

### `set_admin(new_admin)`

Rotates the admin address. The current admin must authorize the call.

### `add_signer(signer)`

Adds a co-signer to the governance set. The admin must authorize the call.

- Fails with `DuplicateSigner` if the address is already registered.
- Fails with `TooManySigners` if adding the signer would exceed `MAX_SIGNERS`.

### `remove_signer(signer)`

Removes a co-signer from the governance set. The admin must authorize the call.

- Fails with `QuorumWouldBreak` if removal would leave fewer signers than the current
  threshold.
- Removing a non-existent signer is a no-op.

### `propose(proposer, target, calldata) -> u32`

Submits a new governance proposal and returns its monotonically increasing ID.

- `proposer` must authorize the call and be a registered co-signer.
- `calldata` is stored as opaque bytes for on-chain auditability.
- `calldata.len()` must be less than or equal to `MAX_CALLDATA_BYTES`.
- The proposer is not automatically counted as an approver.
- Emits `ProposalCreated` with topic `("proposed", proposal_id)`.

### `approve(approver, proposal_id)`

Records an approval from a co-signer.

- `approver` must authorize the call and be a registered co-signer.
- Each signer may approve at most once per proposal.
- Approvals are rejected after execution, cancellation, or expiry.
- Every successful approval emits `ProposalApproved`.
- When the approval count first reaches the configured threshold, the contract stores
  `QuorumInfo { reached_at, threshold }` and emits `QuorumReached`.

### `execute(executor, proposal_id)`

Marks the proposal as executed after quorum and timelock.

- `executor` must authorize the call, but does not need to be a co-signer.
- Execution requires the approval count to satisfy the threshold snapshotted in
  `QuorumInfo`.
- Execution requires
  `env.ledger().timestamp() >= quorum_info.reached_at + GOVERNANCE_TIMELOCK_SECONDS`.
- Execution is rejected after cancellation or expiry.
- The proposal is marked executed and saved before `ProposalExecuted` is emitted.
- Emits `ProposalExecuted` with the stored `target` and `calldata`.

### `cancel_proposal(caller, proposal_id)`

Cancels a proposal, marking it as terminal. Emits `ProposalCancelled`.

- `caller` must be the original proposer or the contract admin.
- Once cancelled, the proposal cannot be approved or executed.
- Calling `cancel_proposal` on an already-cancelled proposal returns `ProposalCancelled`.

### Query entrypoints

- `get_proposal(proposal_id) -> Proposal`: reads the stored proposal.
- `proposal_count() -> u32`: returns the number of proposals created so far.
- `get_signers() -> Vec<Address>`: returns the registered co-signers.
- `quorum() -> u32`: returns the configured approval threshold.
- `timelock_seconds() -> u64`: returns `GOVERNANCE_TIMELOCK_SECONDS`.
- `max_proposal_age_seconds() -> u64`: returns `MAX_PROPOSAL_AGE_SECONDS`.

## Calldata encoding contract

`calldata: Bytes` is intentionally opaque to `FluxoraGovernance`. The contract stores the
bytes, emits them in `ProposalExecuted`, and enforces only the size bound
`MAX_CALLDATA_BYTES = 4,096`. It does not decode function names, validate target ABI, or
perform a generic runtime call into `target`.

The recommended client contract for calldata is:

1. Encode the intended downstream operation deterministically, including target entrypoint
   and arguments. For example, an executor adapter may define `set_cap(i128)` as a small
   tagged payload, or use a fixed XDR schema shared by wallets, indexers, and bots.
2. Show the decoded intent to signers before `propose` and `approve`.
3. Persist the exact bytes in the proposal. Indexers should treat the bytes emitted by
   `ProposalExecuted` as the audited payload that must match the proposal record.
4. Have the off-chain bot or typed on-chain adapter decode the bytes and decide whether to
   call the `target` contract.

Security boundary: a successful `execute` call records governance consensus and emits an
auditable payload. It does not prove that the downstream factory or stream change has
already happened unless a separate adapter transaction or typed dispatch performs that call
and emits its own event.

## Integration with the factory

The `FluxoraFactory` contract stores `max_deposit`, `min_duration`, the recipient allowlist,
and the stream contract address as admin-mutable parameters. To route parameter changes
through governance:

1. Transfer factory admin to the governance contract address or to a typed adapter controlled
   by governance.
2. Encode the desired factory call, such as `set_cap(100_000)`, in `calldata`.
3. After the governance proposal is executed and `ProposalExecuted` is emitted, an authorized
   execution bot or typed adapter decodes `calldata` and applies the change to the factory.

Soroban does not provide a generic arbitrary-calldata invocation primitive inside this
contract. A fully on-chain execution path must be implemented as typed dispatch code, for
example by importing `FluxoraFactoryClient` and matching on a known operation tag.

## Events

For stream-level events, see [`events.md`](events.md). Governance emits the following
proposal events:

| Event | Topic | Payload | Emitted when |
|---|---|---|---|
| `ProposalCreated` | `("proposed", proposal_id)` | `ProposalCreated { proposal_id, proposer, target }` | `propose` stores a new proposal |
| `ProposalApproved` | `("approved", proposal_id)` | `ProposalApproved { proposal_id, approver, approval_count }` | `approve` records a unique signer approval |
| `QuorumReached` | `("quorum", proposal_id)` | `QuorumReached { proposal_id, quorum_reached_at, executable_after }` | Approval count first equals the configured threshold |
| `ProposalCancelled` | `("cancelled", proposal_id)` | `ProposalCancelled { proposal_id, canceller }` | A proposer or admin cancels a proposal |
| `ProposalExecuted` | `("executed", proposal_id)` | `ProposalExecuted { proposal_id, executor, target, calldata }` | `execute` marks the proposal executed after quorum and timelock |

`QuorumReached` is emitted only once per proposal because the contract stores `QuorumInfo`
only when `approval_count == threshold`.

## Storage layout

All storage keys are defined in `DataKey`:

| Key | Storage tier | Type |
|---|---|---|
| `Admin` | Instance | `Address` |
| `Signers` | Instance | `Vec<Address>` |
| `Threshold` | Instance | `u32` |
| `NextProposalId` | Instance | `u32` |
| `Proposal(u32)` | Persistent | `Proposal` (includes `created_at`, `executed`, and `cancelled`) |
| `QuorumReachedAt(u32)` | Persistent | `QuorumInfo { reached_at: u64, threshold: u32 }` |

## GovernanceError codes

For stream and factory error tables, see [`error.md`](error.md). Governance clients should
handle these discriminants from `contracts/governance/src/lib.rs`:

| Error | Code | Typical source | Client guidance |
|---|---:|---|---|
| `NotInitialized` | 1 | Any entrypoint that reads admin or signers before `init` | Block governance actions until deployment calls `init(admin, signers, threshold)`. |
| `AlreadyInitialized` | 2 | Second `init` call | Treat as an operator/configuration mistake; read current state instead of retrying. |
| `Unauthorized` | 3 | Reserved for admin-auth failures in the error enum | Missing admin auth normally fails at `require_auth`; clients should still map this code if an adapter surfaces it. |
| `NotASigner` | 4 | `propose` or `approve` from an address absent from `Signers` | Ask an admin to add the address or switch to a registered co-signer wallet. |
| `ProposalNotFound` | 5 | `get_proposal`, `approve`, `execute`, or `cancel_proposal` with an unknown ID | Refresh proposal lists and verify the ID came from a `ProposalCreated` event. |
| `AlreadyExecuted` | 6 | `approve`, `execute`, or `cancel_proposal` after `proposal.executed = true` | Stop collecting approvals and show the executed state. |
| `QuorumNotReached` | 7 | `execute` before enough approvals, or missing `QuorumInfo` | Continue collecting signer approvals until `approval_count >= threshold`. |
| `TimelockNotElapsed` | 8 | `execute` before `quorum_info.reached_at + GOVERNANCE_TIMELOCK_SECONDS` | Display `executable_after` from `QuorumReached` and retry after that timestamp. |
| `AlreadyApproved` | 9 | Same signer calls `approve` twice for one proposal | Treat the signer as already counted; do not request another approval from that address. |
| `CalldataTooLarge` | 10 | `propose` with `calldata.len() > MAX_CALLDATA_BYTES` | Compress or simplify the encoded operation, or split it into smaller proposals. |
| `TooManySigners` | 11 | `init` or `add_signer` would exceed `MAX_SIGNERS` | Remove an old signer first or deploy governance with a smaller signer set. |
| `ProposalExpired` | 12 | `approve` or `execute` after `MAX_PROPOSAL_AGE_SECONDS` | Treat the proposal as terminal and create a new proposal if the action is still needed. |
| `ProposalCancelled` | 13 | `approve`, `execute`, or repeated cancellation after cancellation | Treat the proposal as terminal and stop collecting approvals. |
| `NotProposerOrAdmin` | 14 | `cancel_proposal` from an address that is neither proposer nor admin | Ask the proposer or admin to cancel, or continue the proposal flow. |
| `InvalidThreshold` | 15 | `init` threshold is zero or exceeds signer count | Choose a threshold in the range `1..=signers.len()`. |
| `QuorumWouldBreak` | 16 | `remove_signer` would leave fewer signers than threshold | Lower the threshold through a governed migration or keep enough signers registered. |
| `DuplicateSigner` | 17 | `init` or `add_signer` includes an already-registered signer | Remove duplicate entries before submitting. |

## Security considerations

1. **No self-approval shortcut**: The proposer must call `approve` separately.
2. **Duplicate approval prevention**: Each signer may approve at most once per proposal.
3. **Duplicate signer prevention**: A co-signer address can only occupy one signer slot.
4. **Timelock starts at quorum**: `QuorumInfo` is written when the approval count first
   equals the configured threshold, not when the proposal is created.
5. **Additional approvals do not reset the clock**: approvals above threshold do not rewrite
   `QuorumInfo`.
6. **Timelock protects against rushed execution**: even with instant quorum, changes cannot
   be executed for at least `GOVERNANCE_TIMELOCK_SECONDS` (48 h).
7. **Executed proposals are immutable**: once `executed = true`, no further approvals or
   re-execution are possible.
8. **Cancelled and expired proposals are terminal**: they cannot be revived, approved, or
   executed.
9. **Cancel authority is restricted**: only the original proposer or the contract admin may
   cancel a proposal.
10. **Admin cannot bypass the process**: the admin can add/remove signers and rotate the
    admin key, but parameter changes still require quorum and timelock.
11. **Calldata is an audit payload**: the governance contract does not decode or enforce
    target-specific calldata semantics.
12. **CEI ordering in `execute`**: the proposal is marked executed and persisted before
    `ProposalExecuted` is emitted.
13. **Threshold invariant prevents governance bricking**: `remove_signer` enforces
    `signers.len() - 1 >= threshold`, so the signer set can never shrink below the required
    approval threshold.
14. **Threshold is snapshotted at quorum time**: execution uses the threshold recorded in
    `QuorumInfo`, so an admin cannot raise or lower the live threshold after quorum to change
    the outcome of an in-flight proposal.

## Tests

Integration tests are in `contracts/stream/tests/governance_integration.rs` and cover:

- Initialization and constant verification.
- Duplicate signer rejection during initialization and signer management.
- Proposal creation and ID assignment.
- Approval counting and duplicate rejection.
- Non-signer rejection on both `propose` and `approve`.
- Quorum enforcement and exact-threshold execution.
- Timelock enforcement.
- Full happy path: propose, two-of-three approve, wait, execute.
- Double-execution prevention.
- Signer management with add/remove.
- Calldata preservation.
- Cancellation by proposer and admin.
- Unauthorized cancellation rejection.
- Double-cancel prevention.
- Cancel of executed proposal prevention.
- Cancel before quorum makes a proposal non-approvable and non-executable.
- Cancel after quorum but before timelock makes a proposal non-executable.
- Expired proposal rejection on approve and execute.
- Expiry boundary behavior.
- Maximum age constant query.
- Threshold validation on `init`.
- Quorum invariant on `remove_signer`.
- Quorum uses the configured threshold; adding signers does not change threshold.
