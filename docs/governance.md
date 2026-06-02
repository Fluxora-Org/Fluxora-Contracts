# Governance Contract

## Purpose

The `FluxoraGovernance` contract (`contracts/governance/src/lib.rs`) implements a minimal
proposal / approve / execute governance pattern with a configurable quorum and timelock.
It decouples operational signing keys from protocol-parameter authority: no single key can
change factory parameters immediately; a quorum of co-signers must approve and a mandatory
waiting period must elapse before the change takes effect.

## Constants

| Constant | Value | Meaning |
|---|---|---|
| `GOVERNANCE_QUORUM` | 2 | Minimum approvals required before execution is allowed |
| `GOVERNANCE_TIMELOCK_SECONDS` | 172 800 (48 h) | Seconds to wait after quorum before executing |
| `MAX_SIGNERS` | 20 | Maximum co-signers registered at once |
| `MAX_CALLDATA_BYTES` | 4 096 | Maximum byte length for the `calldata` field |

## Roles

### Admin
Set at `init` time. The admin can:
- Add / remove co-signers (`add_signer`, `remove_signer`).
- Rotate the admin address (`set_admin`).

The admin alone cannot execute proposals — they must participate as a co-signer if they want
their vote counted toward quorum.

### Co-signers
A fixed set of addresses registered by the admin. Co-signers can:
- Submit proposals (`propose`).
- Approve existing proposals (`approve`).

## Lifecycle of a proposal

```
propose()  →  [0 approvals]
approve()  →  [1 approval]
approve()  →  [quorum reached: timelock starts]
...wait GOVERNANCE_TIMELOCK_SECONDS...
execute()  →  [executed = true, ProposalExecuted event emitted]
```

### State transitions

```
Created → Approved (partial) → Quorum reached → Timelock elapsed → Executed
```

There is no Rejected or Cancelled state; proposals that do not accumulate quorum simply
remain in storage and cannot be executed.

## Entrypoints

### `init(admin, signers)`

Initialises the contract. Can only be called once.

### `propose(proposer, target, calldata) -> u32`

Submits a new governance proposal and returns its monotonically increasing ID.

- `proposer` must be a registered co-signer.
- `calldata` is stored as opaque bytes for on-chain auditability. Its interpretation
  (e.g. which factory setter to call and with what argument) is left to the off-chain
  executor layer.
- The proposer is **not** automatically counted as an approver.

### `approve(approver, proposal_id)`

Records an approval from a co-signer.

- Each signer may approve at most once per proposal.
- When the approval count first reaches `GOVERNANCE_QUORUM`, the timelock clock starts
  and a `QuorumReached` event is emitted.

### `execute(executor, proposal_id)`

Marks the proposal as executed and emits `ProposalExecuted`.

- Any address may call `execute`; `executor` need not be a co-signer.
- Execution requires:
  1. `approvals.len() >= GOVERNANCE_QUORUM`
  2. `current_time >= quorum_reached_at + GOVERNANCE_TIMELOCK_SECONDS`
- The `ProposalExecuted` event contains `target` and `calldata` so that off-chain
  bots or authorised executors can apply the change to the factory.

## Integration with the factory

The `FluxoraFactory` contract stores `max_deposit`, `min_duration`, and the allowlist as
admin-mutable parameters. To route parameter changes through governance:

1. Transfer factory admin to the governance contract address.
2. Encode the desired factory call (e.g. `set_cap(100_000)`) in the `calldata` field.
3. After the governance proposal is executed and `ProposalExecuted` is emitted, the
   factory setter is called by an authorised execution bot that decodes `calldata` and
   calls the factory on behalf of the governance contract.

> **Note**: Soroban does not support generic arbitrary-calldata invocations at runtime.
> For a fully on-chain execution path, extend the `execute` entrypoint to import the
> `FluxoraFactoryClient` and dispatch a typed call based on the decoded `calldata`.

## Events

| Event | Topic | Payload |
|---|---|---|
| `ProposalCreated` | `("proposed", proposal_id)` | proposer, target |
| `ProposalApproved` | `("approved", proposal_id)` | approver, approval_count |
| `QuorumReached` | `("quorum", proposal_id)` | quorum_reached_at, executable_after |
| `ProposalExecuted` | `("executed", proposal_id)` | executor, target, calldata |

## Storage layout

All storage keys are defined in `DataKey`:

| Key | Storage tier | Type |
|---|---|---|
| `Admin` | Instance | `Address` |
| `Signers` | Instance | `Vec<Address>` |
| `NextProposalId` | Instance | `u32` |
| `Proposal(u32)` | Persistent | `Proposal` |
| `QuorumReachedAt(u32)` | Persistent | `u64` (timestamp) |

## Security considerations

1. **No self-approval shortcut**: The proposer must call `approve` separately.
2. **Duplicate approval prevention**: Each signer may approve at most once per proposal.
3. **Timelock protects against rushed execution**: Even with instant quorum, changes
   cannot take effect for at least `GOVERNANCE_TIMELOCK_SECONDS` (48 h).
4. **Executed proposals are immutable**: Once `executed = true`, no further approvals or
   re-execution are possible.
5. **Admin cannot bypass the process**: The admin can only add/remove signers and rotate
   the admin key; parameter changes still require quorum.
6. **CEI ordering in `execute`**: The proposal is marked as executed and state is written
   before the `ProposalExecuted` event, preventing re-entrancy from the event handler.

## Tests

Integration tests are in `contracts/stream/tests/governance_integration.rs` and cover:

- Initialization and constant verification
- Proposal creation and ID assignment
- Approval counting and duplicate rejection
- Non-signer rejection on both propose and approve
- Quorum enforcement (fails with only 1 of 2 required approvals)
- Timelock enforcement (fails before TIMELOCK seconds have elapsed)
- Full happy-path flow (propose → 2-of-3 approve → wait → execute)
- Double-execution prevention
- Signer management (add / remove)
- Calldata preservation
