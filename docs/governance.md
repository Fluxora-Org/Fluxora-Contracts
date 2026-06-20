# Governance Contract

## Purpose

The `FluxoraGovernance` contract (`contracts/governance/src/lib.rs`) implements a
configurable-threshold proposal / approve / execute governance pattern with a timelock.
It decouples operational signing keys from protocol-parameter authority: no single key can
change factory parameters immediately; a threshold of co-signers must approve and a mandatory
waiting period must elapse before the change takes effect.

## Threshold model

The approval **threshold** is set at `init` time and stored in instance storage. It
represents the minimum number of co-signer approvals required before a proposal can be
executed. The invariant `1 <= threshold <= signers.len()` is enforced at:

- `init` — the initial threshold must be between 1 and the initial signer count.
- `remove_signer` — removal is rejected with `QuorumWouldBreak` if it would leave fewer
  signers than the current threshold.
- `add_signer` — the threshold is unchanged (adding signers can never violate the invariant).

When quorum is first reached, the current threshold is **snapshotted** alongside the
timestamp in a `QuorumInfo` record. At execution time the proposal is judged against this
snapshot, making in-flight proposals immune to mid-flight threshold changes by the admin.

## Constants

| Constant | Value | Meaning |
|---|---|---|
| `GOVERNANCE_TIMELOCK_SECONDS` | 172 800 (48 h) | Seconds to wait after quorum before executing |
| `MAX_PROPOSAL_AGE_SECONDS` | 2 592 000 (30 d) | Max age of a proposal before it expires and becomes non-executable |
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

Each co-signer address may appear only once. `init` and `add_signer` reject duplicate
addresses with `DuplicateSigner` so quorum calculations are based on unique keys.

## Lifecycle of a proposal

```
propose()     →  [0 approvals]
approve()     →  [1 approval]
approve()     →  [quorum reached: timelock starts]
...wait GOVERNANCE_TIMELOCK_SECONDS...
execute()     →  [executed = true, ProposalExecuted event emitted]

cancel_proposal()  →  [cancelled = true, ProposalCancelled event emitted]
...after MAX_PROPOSAL_AGE_SECONDS...  →  [proposal expires, no action possible]
```

### State transitions

```
                  ┌─→ Cancelled (terminal)
                  │
Created → Approved (partial) → Quorum reached → Timelock elapsed → Executed
    │                                                            │
    └─→ Expired (terminal, after MAX_PROPOSAL_AGE_SECONDS) ─────┘
```

A proposal enters the **Expired** state automatically when `MAX_PROPOSAL_AGE_SECONDS` have
passed since its creation. Neither approval nor execution is possible on an expired proposal.
The **Cancelled** state is entered explicitly via `cancel_proposal` by the proposer or admin.
Both states are terminal: once a proposal is cancelled or expired, it can never be approved
or executed.

## Entrypoints

### `init(admin, signers, threshold)`

Initialises the contract with an admin, a list of co-signers, and an approval threshold.
`threshold` must satisfy `1 <= threshold <= signers.len()`. Can only be called once.

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
- When the approval count first reaches the configured threshold, the timelock clock starts
  and a `QuorumReached` event is emitted.  The threshold at that moment is snapshotted in
  the `QuorumInfo` record so that in-flight proposals are protected against threshold
  changes.

### `execute(executor, proposal_id)`

Marks the proposal as executed and emits `ProposalExecuted`.

- Any address may call `execute`; `executor` need not be a co-signer.
- Execution requires:
  1. `approvals.len() >= threshold` (using the threshold snapshotted at quorum time)
  2. `current_time >= quorum_reached_at + GOVERNANCE_TIMELOCK_SECONDS`
- The `ProposalExecuted` event contains `target` and `calldata` so that off-chain
  bots or authorised executors can apply the change to the factory.

### `proposal_count() -> u32`

Returns the number of proposals created so far. Proposal IDs are assigned densely
from `0`, so off-chain indexers can enumerate proposals by scanning
`0..proposal_count()` and calling `get_proposal(proposal_id)` for each ID.

### `cancel_proposal(caller, proposal_id)`

Cancels a proposal, marking it as terminal. Emits `ProposalCancelled`.

- `caller` must be the original `proposer` or the contract `admin`.
- Once cancelled, the proposal cannot be approved or executed.
- Idempotent guard: calling `cancel_proposal` on an already-cancelled proposal returns
  `ProposalCancelled`.

### `max_proposal_age_seconds() -> u64`

Returns the `MAX_PROPOSAL_AGE_SECONDS` constant.

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
| `ProposalCancelled` | `("cancelled", proposal_id)` | canceller |
| `ProposalExecuted` | `("executed", proposal_id)` | executor, target, calldata |

### `remove_signer(signer)`

Removes a co-signer from the governance set.  Rejects removal with `QuorumWouldBreak`
if the resulting signer count would fall below the configured threshold, preventing
governance bricking.

### `quorum() -> u32`

Returns the configured approval threshold (not a fixed constant).  The threshold is
set at `init` time and stored in instance storage.

## Storage layout

All storage keys are defined in `DataKey`:

| Key | Storage tier | Type |
|---|---|---|
| `Admin` | Instance | `Address` |
| `Signers` | Instance | `Vec<Address>` |
| `Threshold` | Instance | `u32` |
| `NextProposalId` | Instance | `u32` |
| `Proposal(u32)` | Persistent | `Proposal` (includes `cancelled: bool`) |
| `QuorumReachedAt(u32)` | Persistent | `QuorumInfo { reached_at: u64, threshold: u32 }` |

## Security considerations

1. **No self-approval shortcut**: The proposer must call `approve` separately.
2. **Duplicate approval prevention**: Each signer may approve at most once per proposal.
3. **Duplicate signer prevention**: A co-signer address can only occupy one signer slot.
4. **Timelock protects against rushed execution**: Even with instant quorum, changes
   cannot take effect for at least `GOVERNANCE_TIMELOCK_SECONDS` (48 h).
5. **Executed proposals are immutable**: Once `executed = true`, no further approvals or
   re-execution are possible.
6. **Admin cannot bypass the process**: The admin can only add/remove signers and rotate
   the admin key; parameter changes still require quorum.
7. **CEI ordering in `execute`**: The proposal is marked as executed and state is written
   before the `ProposalExecuted` event, preventing re-entrancy from the event handler.
7. **Proposal expiry prevents latent execution**: Once `MAX_PROPOSAL_AGE_SECONDS` (30 d)
   have elapsed since creation, a proposal becomes expired and cannot be approved or
   executed. This prevents forgotten or abandoned proposals from being used as attack
   vectors.
8. **Cancel authority is restricted**: Only the original proposer or the contract admin
   may cancel a proposal. No other signer can unilaterally cancel a proposal they disagree
   with.
9. **Terminal states are permanent**: A cancelled or expired proposal can never be revived.
   Approvals, re-cancellation, and execution all fail on proposals in terminal states.
10. **Threshold invariant prevents governance bricking**: `remove_signer` enforces
    `signers.len() - 1 >= threshold`, so the signer set can never shrink below the
    required approval threshold.  This guarantees quorum is always attainable.
11. **Threshold is snapshotted at quorum time**: When quorum is first reached, the
    current threshold is recorded inside `QuorumInfo`.  Execution uses this snapshot
    rather than the live threshold, so an admin cannot raise the threshold after quorum
    is reached to block execution, nor lower it to let a proposal with fewer approvals
    through.
12. **`init` validates threshold bounds**: The threshold must be `>= 1` and
    `<= signers.len()`, preventing degenerate configurations at deployment.

## Tests

Integration tests are in `contracts/stream/tests/governance_integration.rs` and cover:

- Initialization and constant verification
- Duplicate signer rejection during initialization and signer management
- Proposal creation and ID assignment
- Approval counting and duplicate rejection
- Non-signer rejection on both propose and approve
- Quorum enforcement (fails with only 1 of 2 required approvals)
- Timelock enforcement (fails before TIMELOCK seconds have elapsed)
- Full happy-path flow (propose → 2-of-3 approve → wait → execute)
- Double-execution prevention
- Signer management (add / remove)
- Calldata preservation
- **Cancellation by proposer and admin**
- **Unauthorized cancellation rejection**
- **Double-cancel prevention**
- **Cancel of executed proposal prevention**
- **Cancel before quorum makes proposal non-approvable / non-executable**
- **Cancel after quorum but before timelock makes proposal non-executable**
- **Expired proposal rejection on approve and execute**
- **Expiry boundary (exact boundary behaviour, 1 second past boundary)**
- **Max age constant query**
- **Threshold validation on init** (zero, above signer count, at boundary, minimum)
- **Quorum invariant on `remove_signer`** (removal down to threshold succeeds, removal
  below threshold errors, non-existent removal is no-op)
- **Quorum uses configured threshold** (adding signers does not change threshold)
- **Execution with exactly threshold approvals**
