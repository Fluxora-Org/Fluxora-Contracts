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
- `set_threshold`: the new threshold must be between 1 and the current signer count.
- `remove_signer`: removal is rejected with `QuorumWouldBreak` if it would leave fewer
  signers than the current threshold.
- `add_signer`: the threshold is unchanged, so adding signers can never violate the invariant.

**Design choice: reject removal vs auto-adjust threshold**

When removing a signer would violate the threshold invariant (i.e., `signers.len() - 1 < threshold`),
the contract **rejects the removal** with `QuorumWouldBreak` rather than automatically adjusting
the threshold downward. This design choice was made for the following reasons:

1. **Explicit intent**: Threshold changes should be deliberate, not side effects of signer removal.
2. **Auditability**: Requiring a separate `set_threshold` call creates a clear audit trail in events.
3. **Security**: Auto-adjusting could silently lower security requirements; rejection forces admin awareness.
4. **Reversibility**: If removal is rejected, the admin can first lower the threshold via `set_threshold`,
   then remove the signer, maintaining full control over the sequence of changes.

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
| `MAX_PAGE_SIZE` | 100 | Maximum proposals returned by `get_proposals_by_id_range` per call |

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

- Emits `AdminChanged` with topic `("adm_chg",)` carrying the previous and new admin.

### `add_signer(signer)`

Adds a co-signer to the governance set. The admin must authorize the call.

- Fails with `DuplicateSigner` if the address is already registered.
- Fails with `TooManySigners` if adding the signer would exceed `MAX_SIGNERS`.
- Emits `SignerAdded` with topic `("sgnr_add",)` after the signer set is persisted.

### `remove_signer(signer)`

Removes a co-signer from the governance set. The admin must authorize the call.

- Fails with `QuorumWouldBreak` if removal would leave fewer signers than the current
  threshold.
- Removing a non-existent signer is a no-op and emits **no** event.
- Emits `SignerRemoved` with topic `("sgnr_rm",)` only when a registered signer is
  actually removed.
- Stale approvals: Removing a co-signer does not mutate historical proposal records, but approvals from removed signers are filtered out at evaluation time during `approve`, `execute`, and `is_executable`. A proposal that relies on a removed signer's approval to reach threshold cannot be executed unless sufficient active co-signers approve.

### `set_threshold(threshold)`

Sets the approval threshold for governance proposals. The admin must authorize the call.

- Fails with `InvalidThreshold` unless `1 <= threshold <= current_signer_count`.
- Emits `QuorumConfig` with topic `("quor_cfg",)` carrying the new threshold and signer count.
- The threshold change does not affect in-flight proposals, which are judged against the
  threshold snapshotted when they first reached quorum.

### `propose(proposer, target, calldata) -> u32`

Submits a new governance proposal and returns its monotonically increasing ID.

- `proposer` must authorize the call and be a registered co-signer.
- `calldata` is stored as opaque bytes for on-chain auditability.
- `calldata` must be non-empty (at least 1 byte); empty calldata is rejected with `CalldataEmpty`.
- `calldata.len()` must be less than or equal to `MAX_CALLDATA_BYTES`.
- The proposer is not automatically counted as an approver.
- Emits `ProposalCreated` with topic `("proposed", proposal_id)`.
- Rejected proposals (empty or oversized calldata, non-signer) do not consume a proposal ID.

### `approve(approver, proposal_id)`

Records an approval from a co-signer.

- `approver` must authorize the call and be a registered co-signer.
- Each signer may approve at most once per proposal.
- Approvals are rejected after execution, cancellation, or expiry.
- Every successful approval emits `ProposalApproved`.
- When the approval count first reaches the configured threshold, the contract stores
  `QuorumInfo { reached_at, threshold }` and emits `QuorumReached`.

### `execute(executor, proposal_id)`

Marks the proposal as executed after quorum and timelock, then dispatches the
encoded `CallData` operation to the `target` contract.

- `executor` must authorize the call, but does not need to be a co-signer.
- Execution requires the approval count to satisfy the threshold snapshotted in
  `QuorumInfo`.
- Execution requires
  `env.ledger().timestamp() >= quorum_info.reached_at + GOVERNANCE_TIMELOCK_SECONDS`.
- Execution is rejected after cancellation or expiry.
- The proposal is marked executed and saved before the cross-contract dispatch
  (CEI ordering); if the dispatch panics the transaction reverts, restoring the
  un-executed state so the call can be retried.
- After a successful dispatch, emits `ProposalExecuted` with the stored `target`
  and `calldata`.
- Returns `GovernanceError::InvalidCalldata` (19) if the calldata bytes deserialise
  but do not represent a known `CallData` variant.

### `cancel_proposal(caller, proposal_id)`

Cancels a proposal, marking it as terminal. Emits `ProposalCancelled`.

- `caller` must be the original proposer or the contract admin.
- Once cancelled, the proposal cannot be approved or executed.
- Calling `cancel_proposal` on an already-cancelled proposal returns `ProposalCancelled`.

### Query entrypoints

- `get_proposal(proposal_id) -> Proposal`: reads the stored proposal.
- `proposal_count() -> u32`: returns the number of proposals created so far.
- `get_signers() -> Vec<Address>`: returns the registered co-signers.
- `get_admin() -> Result<Address, GovernanceError>`: returns the admin address.
  Returns `NotInitialized` if `init` has not been called.
- `get_threshold() -> Result<u32, GovernanceError>`: returns the configured
  approval threshold.  Returns `NotInitialized` if `init` has not been called.
  For a non-erroring convenience wrapper (returns `0` when uninitialized), use
  `quorum()`.
- `quorum() -> u32`: returns the configured approval threshold (returns `0` if
  uninitialized).  Convenience alias for `get_threshold()`.
- `timelock_seconds() -> u64`: returns `GOVERNANCE_TIMELOCK_SECONDS`.
- `max_proposal_age_seconds() -> u64`: returns `MAX_PROPOSAL_AGE_SECONDS`.
- `get_quorum_info(proposal_id) -> Option<QuorumInfo>`: returns the stored
  `QuorumInfo { reached_at, threshold }` snapshot if quorum was reached, or
  `None` if quorum has not yet been reached.  No authorization required.
- `is_executable(proposal_id) -> bool`: returns `true` iff the proposal
  exists, is not cancelled/executed/expired, quorum is reached, and
  `now >= reached_at + GOVERNANCE_TIMELOCK_SECONDS`.  Mirrors the exact
  gating order used by `execute`.  Returns an error (`ProposalNotFound` or
  `ArithmeticOverflow`) only when `execute` would also error.  No
  authorization required.
- `is_signer(signer) -> bool`: returns `true` iff `signer` is a registered
  co-signer.  Cheap O(1) membership probe over `DataKey::SignerIndex`
  (the same `Map<Address, bool>` consulted internally by `propose`,
  `approve`, `add_signer`, and `remove_signer`).  Lets off-chain tooling
  and cross-contract callers verify membership without iterating the
  full signer list (which is O(n) on the wire and on the receiving side).
  Returns `false` (no panic, no error) when the contract has not been
  initialised, so callers can use it as a safe membership probe without
  first having to check initialisation via `get_admin`.  Pure read — no
  authorization, no state mutation, no TTL extension.
- `get_proposals_by_id_range(start_id, limit) -> Vec<Proposal>`: returns a
  bounded page of proposals — see [Paginated enumeration](#paginated-enumeration)
  below.

## Paginated enumeration

### `get_proposals_by_id_range(start_id: u32, limit: u32) -> Vec<Proposal>`

Returns proposals whose IDs fall in the half-open range `[start_id, start_id + limit)`,
capped at `MAX_PAGE_SIZE = 100` entries per call.

#### Behaviour

| Condition | Result |
|---|---|
| `limit == 0` | Empty `Vec` |
| `start_id >= proposal_count()` | Empty `Vec` |
| `limit > MAX_PAGE_SIZE` | Silently clamped to `MAX_PAGE_SIZE` |
| ID is missing from storage | Skipped without error |
| ID is present (any status) | Included in result |

Proposal IDs are assigned monotonically from 0.  The effective iteration range
is `[start_id, min(start_id + effective_limit, proposal_count()))`.

#### DoS protection

`limit` is hard-capped at `MAX_PAGE_SIZE` (100) before any storage reads.
Callers cannot exceed this by any means — passing `u32::MAX` is safe and
equivalent to passing `100`.  At 100 proposals per page the read budget stays
within Soroban's metered limits.

#### Missing IDs

Proposals that are present in storage (including cancelled and executed
proposals) are included.  IDs that were never written — or that were
hypothetically rolled back — are silently skipped.  The returned slice
preserves ascending ID order.

> **Note**: `cancel_proposal` marks `cancelled = true` but **does not**
> delete the storage entry, so cancelled proposals continue to appear in
> range queries.  Likewise, executed proposals appear with `executed = true`.

#### Pagination pattern

```rust
// Page through all proposals in batches of 100:
let page_size: u32 = 100;
let mut start: u32 = 0;
loop {
    let page = client.get_proposals_by_id_range(&start, &page_size);
    if page.is_empty() {
        break;
    }
    process(page);
    start += page_size;
}
```

#### Example (single page)

```rust
// Retrieve proposals 0-49 (up to 50 results):
let proposals = client.get_proposals_by_id_range(&0, &50);

// Retrieve proposals 100-199 (up to 100 results, clamped at MAX_PAGE_SIZE):
let proposals = client.get_proposals_by_id_range(&100, &100);
```

## Calldata encoding contract

`calldata: Bytes` carries a typed, XDR-serialised `CallData` value. Proposers
encode the intended operation by constructing a `CallData` variant and calling
`.to_xdr(&env)` on it; `execute` decodes the bytes with `CallData::from_xdr`
and dispatches the corresponding cross-contract call to `target`.

### Supported operations (`CallData` variants)

| Variant | Target | Downstream call |
|---|---|---|
| `Noop` | — | No call (useful for governance-mechanics-only proposals) |
| `StreamSetAdmin(Address)` | stream contract | `set_admin(new_admin)` |
| `StreamSetMaxRate(i128)` | stream contract | `set_max_rate_per_second(max_rate)` |
| `FactorySetAdmin(Address)` | factory contract | `set_admin(new_admin)` |
| `FactorySetCap(i128)` | factory contract | `set_cap(max_deposit)` |
| `FactorySetMinDuration(u64)` | factory contract | `set_min_duration(min_duration)` |
| `FactorySetAllowlist(Address, bool)` | factory contract | `set_allowlist(recipient, allowed)` |
| `FactorySetStreamContract(Address)` | factory contract | `set_stream_contract(new_contract)` |

### Encoding example (Rust)

```rust
use soroban_sdk::xdr::ToXdr;
use fluxora_governance::CallData;

// Encode a factory cap change to 100_000 units.
let calldata = CallData::FactorySetCap(100_000_i128).to_xdr(&env);
governance_client.propose(&proposer, &factory_address, &calldata);
```

### Worked example: routing a stream rate change

`set_max_rate_per_second` on the stream contract is admin-only, so once the
stream contract's admin has been transferred to governance (see
[Integration](#integration-with-the-factory-and-stream-contracts)) the rate can
only be changed through a proposal. Encode the change as a `StreamSetMaxRate`
variant:

```rust
use soroban_sdk::xdr::ToXdr;
use fluxora_governance::CallData;

// Raise the per-second rate cap on the stream contract to 5_000 units.
let calldata = CallData::StreamSetMaxRate(5_000_i128).to_xdr(&env);
let proposal_id = governance_client.propose(&proposer, &stream_address, &calldata);
```

On `execute`, the contract decodes the bytes back into
`CallData::StreamSetMaxRate(5_000)` and dispatches
`set_max_rate_per_second(5_000)` to `stream_address`. The emitted
`ProposalExecuted { proposal_id, executor, target, calldata }` event carries the
exact `calldata` bytes, so an indexer can decode them with `CallData::from_xdr`
and confirm the applied rate without reading the stream contract's storage.

### Failure modes

| Condition | Behaviour |
|---|---|
| Bytes deserialise as a non-`CallData` ScVal (e.g. a plain `u32`) | `execute` returns `GovernanceError::InvalidCalldata` (19); proposal stays un-executed |
| Completely non-XDR bytes | Host aborts the transaction; proposal state is reverted |
| Target contract rejects the call (e.g. wrong admin) | Host aborts the transaction; proposal state is reverted |

In every failure case the `executed = true` write is rolled back (Soroban atomic
transaction semantics), so a failed execution can be retried after the underlying
cause is resolved.

Security boundary: a successful `execute` call now proves that the downstream
parameter change has been applied on-chain. The `ProposalExecuted` event carries
the original `calldata` bytes, letting indexers verify the dispatched operation
without any side-channel.

### ABI stability of the `CallData` enum

`CallData` is part of the protocol's external ABI: proposers encode against it
and indexers decode `ProposalExecuted.calldata` against it. Because the enum
derives `#[contracttype]`, each variant is serialised with its **name** as the
leading symbol followed by its payload, so the variant *names and payload
shapes* — not their declaration order — are what integrators depend on. Treat
the enum under the frozen-discriminant rules in
[`ABI_STABILITY.md` §5](ABI_STABILITY.md#5-frozen-enum-discriminants) and the
breaking-change taxonomy in
[`ABI_STABILITY.md` §3](ABI_STABILITY.md#3-what-counts-as-a-breaking-change):

| Change to `CallData` | Breaking? | Why |
|---|---|---|
| Append a new variant at the end | No | Existing calldata still decodes. The new name is only recognised by deployments that ship the variant; older deployments return `InvalidCalldata` (19), which is already the documented behaviour for unknown variants. |
| Reorder existing variants | No | Decoding keys off the variant name, not its position. |
| Rename a variant | Yes | Changes the leading symbol, so calldata stored in open proposals and indexer decoders keyed on the old name stop matching. |
| Remove a variant | Yes | Any proposal whose stored calldata references it can no longer execute. |
| Change a variant's payload type or arity | Yes | Alters the ScVal shape after the name symbol, so previously-encoded calldata mis-decodes. |

Any change in the "Breaking" rows requires a `CONTRACT_VERSION` increment and a
new deployment, consistent with §3.1.


## Integration with the factory and stream contracts

The `FluxoraFactory` contract stores `max_deposit`, `min_duration`, the recipient allowlist,
and the stream contract address as admin-mutable parameters. `FluxoraStream` exposes
`set_admin` and `set_max_rate_per_second`. To route parameter changes through governance:

1. Transfer the target contract's admin to the governance contract address.
2. Encode the desired operation as a `CallData` variant and serialise it with `.to_xdr(&env)`.
3. Submit a proposal via `propose(proposer, target, calldata)`.
4. Collect the required threshold of `approve` calls and wait for the timelock.
5. Call `execute(executor, proposal_id)`. The governance contract decodes `calldata`,
   calls the target with the encoded arguments, and emits `ProposalExecuted`.

No off-chain bot is required; the parameter change is enforced atomically within the
`execute` transaction.

## Event-driven execution pattern

While the current `execute` implementation dispatches the cross-contract call atomically
inside the transaction, the contract also emits a `ProposalExecuted` event carrying the
`target` address and `calldata` bytes. This enables an **alternative operational pattern**
where an off-chain executor monitors governance events and applies the change
independently.

The `ExecutorStub` in `contracts/stream/tests/governance_executor_e2e.rs` demonstrates
this pattern:

1. An executor reads `ProposalExecuted` topics from the governance contract.
2. It decodes the `target` and `calldata` from the event payload.
3. It deserialises the `CallData` variant from the XDR-encoded `calldata`.
4. It invokes the appropriate function on the target contract (e.g.
   `stream.set_max_rate_per_second`).

In this pattern the target contract's admin must be set to the executor's address (or to a
multisig that the executor controls), not to the governance contract. The governance
proposal serves as the **authority record** — the executor applies the change based on the
provenance recorded in the `ProposalExecuted` event.

The e2e test validates both the atomic dispatch and the event-driven flow:

- propose → approve to quorum → wait timelock → execute → executor reads event → verifies
  state change.
- Pre-quorum / pre-timelock execution is blocked at the governance level.
- Cancelled and expired proposals produce no `ProposalExecuted` event and therefore no
  executor action.


## End-to-End Governance-Stream Integration

The primary purpose of the governance system is to control stream contract parameters through
a secure, multi-signature approval process with mandatory timelock delays. This section
documents the complete integration pattern and security model.

### Stream Admin Model

The stream contract (`FluxoraStream`) has an `admin` address stored in its configuration,
which can be set to the governance contract address to enable governance control. When
governance is the stream admin, all admin-privileged operations require a successful
governance proposal execution:

- `set_max_rate_per_second(max_rate: i128)`: Set governance-controlled rate limits
- `set_admin(new_admin: Address)`: Transfer admin privileges (including back to governance)
- `set_global_emergency_paused(paused: bool)`: Emergency pause controls
- `pause_stream_as_admin(stream_id: u64)`: Admin pause individual streams
- `cancel_stream_as_admin(stream_id: u64)`: Admin cancel streams
- `sweep_excess(recipient: Address)`: Recover excess tokens

### Integration Architecture

```text
┌─────────────┐    propose/approve/execute    ┌──────────────────┐
│ Governance  │◄─────────────────────────────┤ Co-signer Wallets│
│ Contract    │                              └──────────────────┘
└─────┬───────┘
      │ admin relationship
      ▼
┌─────────────┐    set_max_rate_per_second    ┌──────────────────┐
│ Stream      │◄─────────────────────────────┤ Off-chain        │
│ Contract    │        (after execution)      │ Executor Bot     │
└─────────────┘                              └──────────────────┘
```

**Components:**
1. **Governance Contract**: Manages the proposal lifecycle with threshold signatures and timelock
2. **Stream Contract**: Target of governance-controlled parameter changes
3. **Co-signer Wallets**: Multi-signature participants who propose and approve changes
4. **Off-chain Executor**: Monitors `ProposalExecuted` events and applies the changes

### Complete Integration Flow

#### 1. Initial Setup

```rust
// Deploy governance with multi-sig configuration
governance.init(
    admin: Address,           // Admin for signer management  
    signers: Vec<Address>,    // Co-signers (e.g., 3 addresses)
    threshold: u32            // Required approvals (e.g., 2-of-3)
);

// Deploy stream contract with governance as admin
stream.init(
    token: Address,           // Payment token address
    admin: governance_id      // CRITICAL: Governance controls stream
);

// Verify governance control
let config = stream.get_config();
assert_eq!(config.admin, governance_id);
```

#### 2. Parameter Change Proposal

```rust
// Co-signer creates proposal to change max rate from unlimited to 1000/sec
let calldata = encode_stream_call("set_max_rate_per_second", 1000i128);
let proposal_id = governance.propose(
    proposer: Address,        // Must be registered co-signer
    target: stream_id,        // Target contract to modify
    calldata: Bytes           // Encoded parameter change
);

// Emits: ProposalCreated { proposal_id, proposer, target }
```

#### 3. Multi-Signature Approval

```rust
// First co-signer approval
governance.approve(signer_a, proposal_id);
// Emits: ProposalApproved { proposal_id, approver: signer_a, approval_count: 1 }

// Second co-signer approval (reaches 2-of-3 threshold)  
governance.approve(signer_b, proposal_id);
// Emits: ProposalApproved { proposal_id, approver: signer_b, approval_count: 2 }
// Emits: QuorumReached { proposal_id, quorum_reached_at, executable_after }
```

#### 4. Timelock Period

Once quorum is reached, the proposal enters a mandatory 48-hour timelock period:

```rust
let quorum_info = load_quorum_info(proposal_id);
let executable_after = quorum_info.reached_at + GOVERNANCE_TIMELOCK_SECONDS;

// Attempts to execute before timelock fail
assert_eq!(
    governance.try_execute(executor, proposal_id),
    Err(GovernanceError::TimelockNotElapsed)
);
```

This timelock provides a critical security window where:
- The community can review the approved change
- Emergency cancellation is possible if issues are discovered
- Market participants can prepare for the parameter change

#### 5. Proposal Execution

After the timelock elapses, any address can trigger execution:

```rust
// Execute the approved proposal  
governance.execute(executor, proposal_id);
// Emits: ProposalExecuted { 
//     proposal_id, 
//     executor, 
//     target: stream_id, 
//     calldata: "set_max_rate_per_second:1000" 
// }
```

**Critical Security Point:** The governance contract does NOT automatically apply the
parameter change. Execution only:
- Marks the proposal as executed
- Emits the `ProposalExecuted` event with the approved calldata
- Provides an auditable record of governance consensus

#### 6. Off-Chain Parameter Application

An authorized execution bot monitors for `ProposalExecuted` events and applies the changes:

```rust
// Off-chain executor (running as governance contract)
fn handle_proposal_executed(event: ProposalExecuted) {
    // Decode the approved calldata
    let call = decode_stream_call(&event.calldata);
    
    match call {
        StreamCall::SetMaxRatePerSecond(rate) => {
            // Apply the governance-approved change
            stream.set_max_rate_per_second(rate);
            // This succeeds because governance is the stream admin
        }
        // Handle other governance-approved operations...
    }
}
```

#### 7. Parameter Change Verification

The change takes effect immediately when applied:

```rust
// Stream creation now enforces the governance-set rate limit
let result = stream.create_stream(
    sender, recipient, 
    deposit: 10000,
    rate_per_second: 1500,  // > 1000 (governance limit)
    // ... other params
);

// Fails with error indicating rate cap exceeded
assert!(result.is_err()); 

// Stream creation within limits succeeds
let result = stream.create_stream(
    sender, recipient,
    deposit: 5000, 
    rate_per_second: 500,   // <= 1000 (within governance limit)
    // ... other params  
);
assert!(result.is_ok());
```

### Security Model

#### Admin Authorization Chain

```text
1. Co-signers → Governance Proposal (threshold + timelock)
2. Governance Execution → Stream Parameter Change  
3. Stream Enforcement → User Transaction Validation
```

**Key Security Properties:**

1. **No Single Point of Failure**: No individual key can change stream parameters
2. **Transparent Process**: All governance actions are recorded on-chain with events
3. **Time-Delayed Execution**: 48-hour minimum between approval and application
4. **Immutable Audit Trail**: Proposal content and approvals are permanently stored
5. **Cancellation Window**: Proposals can be cancelled during the timelock period

#### Attack Resistance

The governance integration is designed to resist several attack vectors:

**Compromised Co-signer Key:**
- Single compromised key cannot propose AND approve changes alone
- Threshold requirement (2-of-3) means attacker needs multiple keys
- Timelock provides detection and response window

**Rushed Parameter Changes:**  
- Mandatory 48-hour timelock prevents immediate execution
- Community has time to review and potentially cancel harmful proposals
- Emergency cancellation available through admin or original proposer

**Admin Key Compromise:**
- If governance contract key is compromised, attacker still cannot bypass process
- Parameter changes still require full proposal → approval → timelock → execution flow
- Admin can only manage co-signer set, not directly change stream parameters

**Off-chain Executor Compromise:**
- Executor can only apply governance-approved changes
- Cannot modify or forge the approved calldata
- All actions are authenticated through on-chain governance records

### Implementation Requirements

For teams implementing governance-stream integration:

#### 1. Contract Deployment
```rust
// Correct initialization order
let governance_id = deploy_governance(admin, signers, threshold);
let stream_id = deploy_stream(token, governance_id); // governance as admin
```

#### 2. Calldata Encoding
```rust
// Standardized encoding for parameter changes
fn encode_set_max_rate(rate: i128) -> Bytes {
    format!("set_max_rate_per_second:{}", rate).into_bytes()
}

// Must be deterministic and parseable by executor
```

#### 3. Off-chain Monitoring
```rust
// Event monitoring for execution
governance.events()
    .filter(|e| e.topic == ("executed", proposal_id))
    .subscribe(handle_proposal_executed);
```

#### 4. Error Handling
```rust  
// Graceful handling of governance errors
match governance.try_execute(executor, proposal_id) {
    Err(GovernanceError::TimelockNotElapsed) => {
        // Schedule retry after timelock
        schedule_execution(proposal_id, executable_after);
    },
    Err(GovernanceError::QuorumNotReached) => {
        // Continue collecting approvals
        request_additional_approvals(proposal_id);
    },
    // ... handle other cases
}
```

### Testing Requirements

The end-to-end governance-stream integration requires comprehensive testing:

#### Integration Test Coverage
- Deploy both contracts in test environment
- Set governance as stream admin  
- Create parameter change proposals
- Verify multi-signature approval process
- Test timelock enforcement
- Confirm parameter changes take effect
- Validate security boundaries (unauthorized changes fail)

#### Security Test Cases
- Verify unauthorized actors cannot bypass governance
- Test timelock prevents premature execution  
- Confirm cancellation works during timelock period
- Validate proposal expiry after maximum age
- Test admin key rotation through governance

#### Edge Case Validation
- Handle proposal conflicts (multiple rate changes)
- Test governance contract admin key rotation
- Verify behavior when stream admin changes
- Test governance with minimum threshold (1-of-1)
- Validate maximum threshold (all signers required)

### Integration Test Implementation

The complete end-to-end test is located at:
`contracts/stream/tests/governance_integration.rs::test_end_to_end_governance_stream_parameter_change`

This test validates:

1. **Contract Deployment**: Both governance and stream contracts deploy successfully
2. **Admin Relationship**: Stream admin is set to governance contract address
3. **Baseline Verification**: Stream creation works normally before governance changes
4. **Proposal Creation**: Governance proposal for `set_max_rate_per_second` is created
5. **Approval Process**: Multi-signature approval reaches quorum (2-of-3 signers)
6. **Timelock Enforcement**: Execution fails before 48-hour timelock elapses
7. **Successful Execution**: Proposal executes successfully after timelock
8. **Off-chain Application**: Simulates executor applying the governance-approved change
9. **Parameter Enforcement**: Verifies rate cap is enforced in stream creation
10. **Security Validation**: Confirms unauthorized actors cannot bypass governance
11. **Edge Case Testing**: Validates timelock enforcement on subsequent proposals

This test represents the critical integration seam between governance and stream contracts,
proving that the governance system can successfully control stream parameters through the
complete multi-signature, time-delayed approval process.

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

### Membership and admin events

In addition to the proposal lifecycle, governance emits structured events for every
mutation of the co-signer set and the admin address, so indexers can reconstruct the
full signer set and admin history from chain events alone. These topics are
single-element (no `proposal_id`) and are deliberately distinct from the proposal
topics above so they never collide.

| Event | Topic | Payload | Emitted when |
|---|---|---|---|
| `SignerAdded` | `("sgnr_add",)` | `SignerAdded { signer }` | `add_signer` adds a new co-signer (after the signer set is persisted) |
| `SignerRemoved` | `("sgnr_rm",)` | `SignerRemoved { signer }` | `remove_signer` removes a registered co-signer (after the signer set is persisted) |
| `AdminChanged` | `("adm_chg",)` | `AdminChanged { old, new }` | `set_admin` rotates the admin (after the new admin is persisted) |

Emission guarantees and CEI ordering:

- All three events are emitted **after** the corresponding state mutation is persisted,
  following the contract's check-effects-interactions discipline.
- `remove_signer` against an address that is not registered is a silent no-op and emits
  **no** `SignerRemoved` event. Likewise, a rejected `remove_signer` (`QuorumWouldBreak`)
  or a rejected `add_signer` (`DuplicateSigner` / `TooManySigners`) emits no event.
- `AdminChanged` carries both the previous (`old`) and new (`new`) admin so the full
  admin rotation chain is reconstructable without reading historical state.

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

### TTL policy

Soroban persistent entries are subject to archival once their remaining
TTL falls below `PERSISTENT_LIFETIME_THRESHOLD` (17,280 ledgers / ~1 day
at 5 s/ledger). To keep `Proposal(id)` and `QuorumReachedAt(id)` live
throughout the timelock window, the contract bumps TTL on every read and
write that touches the entry:

- **`Proposal(id)`**: bumped via `bump_proposal` in `load_proposal` (read path,
  called by `get_proposal`, `is_executable`, `approve`, `execute`,
  `cancel_proposal`) and in `save_proposal` (write path, called by `propose`,
  `approve`, `cancel_proposal`, `execute`).
- **`QuorumReachedAt(id)`**: bumped when quorum is first reached inside
  `approve` (write path), and also bumped on read by `get_quorum_info`
  and `is_executable` (read path).
- **`ProposalApprovalIdx(id)`**: bumped via `bump_approval_index` in
  `get_approval_index` (read path), `save_approval_index` (write path),
  `load_proposal` (keeps the index coupled to the proposal record TTL),
  and explicitly after every successful `approve`.

Constants:

| Symbol | Value | Purpose |
|---|---:|---|
| `PERSISTENT_LIFETIME_THRESHOLD` | 17,280 ledgers (~1 d) | Soroban archival threshold; entries whose remaining TTL falls below this value are bump-extended. |
| `PERSISTENT_BUMP_AMOUNT` | 120,960 ledgers (~7 d) | Bump amount applied on every read and write of `Proposal(id)`, `ProposalApprovalIdx(id)`, and on `QuorumReachedAt(id)` at quorum-reach. |

The 48-hour timelock corresponds to ~34,560 ledgers, which is comfortably
covered by a single 7-day bump. The 30-day `MAX_PROPOSAL_AGE_SECONDS`
window (~518,400 ledgers) requires periodic reads from clients,
indexers, or admin tools to keep entries alive past the initial ~7-day
bump; the regression tests in `contracts/stream/tests/governance_ttl.rs`
pin this behavior.

Security implication: a future change that removes the read-time bump in
`load_proposal` would cause a `Proposal(id)` entry to archive silently
between reads, turning `execute` into a `ProposalNotFound` failure
surface for in-flight, still-timelocked proposals. The
`test_execute_unknown_id_returns_proposal_not_found` test in
`governance_ttl.rs` documents the failure signal that change would
produce.

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
| `ArithmeticOverflow` | 18 | Timelock or expiry deadline arithmetic would overflow `u64` | This should not occur under normal ledger conditions; treat as a fatal contract error. |
| `CalldataEmpty` | 19 | `propose` called with zero-length calldata | Provide at least one byte of calldata encoding the intended operation. |

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
12. **Non-empty calldata required**: `propose` rejects zero-length calldata with
    `CalldataEmpty` before any state mutation — the proposal ID counter is not incremented
    and no storage is written. This prevents vacuous proposals from entering governance and
    ensures every on-chain proposal carries a verifiable intent payload.
13. **CEI ordering in `execute`**: the proposal is marked executed and persisted before
    `ProposalExecuted` is emitted.
14. **Threshold invariant prevents governance bricking**: `remove_signer` enforces
    `signers.len() - 1 >= threshold`, so the signer set can never shrink below the required
    approval threshold.
15. **Threshold is snapshotted at quorum time**: execution uses the threshold recorded in
    `QuorumInfo`, so an admin cannot raise or lower the live threshold after quorum to change
    the outcome of an in-flight proposal.
16. **Auditable membership and admin changes**: `add_signer`, `remove_signer`, and
    `set_admin` emit `SignerAdded`, `SignerRemoved`, and `AdminChanged` respectively, all
    after the state mutation is persisted (CEI). No membership or admin change is silent,
    so indexers can reconstruct the live signer set and admin history from events. A
    no-op `remove_signer` (unregistered address) and any rejected mutation emit no event,
    so an event presence faithfully implies a real state change.

## Tests

Integration tests are in `contracts/stream/tests/governance_integration.rs` (870 lines)
and `contracts/stream/tests/governance_executor_e2e.rs` (executor-stub e2e) and cover:

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
- Membership and admin events: `add_signer` emits `SignerAdded`, `remove_signer` emits
  `SignerRemoved`, and `set_admin` emits `AdminChanged` with correct old/new across an
  admin rotation chain.

Inline `#[cfg(test)]` tests for `get_proposals_by_id_range` (in `contracts/governance/src/lib.rs`) cover:

- Empty contract — no proposals → empty result for any inputs.
- Zero limit → always empty.
- `start_id` at exactly `proposal_count()` (exclusive bound) → empty.
- `start_id` well beyond `proposal_count()` → empty.
- Full page: all proposals returned in ascending ID order.
- Partial page: `start_id` mid-range returns the tail slice only.
- Limit clamping: `limit > MAX_PAGE_SIZE` is silently clamped; result length ≤ `MAX_PAGE_SIZE`.
- Limit exactly equal to `MAX_PAGE_SIZE` is not rejected.
- Cancelled proposals remain in storage and appear in range results (`cancelled = true`).
- Executed proposals remain in storage and appear in range results (`executed = true`).
- Two-page pagination: consecutive calls with non-overlapping `start_id` cover all IDs.
- `u32::MAX` limit is handled safely via `saturating_add`; no panic, clamped to `MAX_PAGE_SIZE`.
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
- Membership and admin events: `add_signer` emits `SignerAdded`, `remove_signer` emits
  `SignerRemoved`, and `set_admin` emits `AdminChanged` with correct old/new across an
  admin rotation chain. Snapshot assertions verify both topics and payloads.
- Negative event coverage: removing a non-existent signer, a `QuorumWouldBreak`-rejected
  removal, and a `DuplicateSigner`-rejected add all emit no event.

The executor-stub e2e test (`governance_executor_e2e.rs`) adds end-to-end coverage for
the event-driven operational pattern:

- **Full flow**: propose → approve to quorum → wait timelock → execute → stream parameter
  changes (`set_max_rate_per_second`).
- **Executor stub decodes `ProposalExecuted`**: reads events, decodes `target` +
  `calldata`, and independently dispatches the call.
- **Pre-quorum/pre-timelock blocked**: parameter change is impossible before quorum or
  timelock elapses.
- **Cancelled/expired proposals**: no `ProposalExecuted` event emitted, no executor
  action.
- **Security invariant**: the parameter change requires the complete quorum + timelock +
  execute path; no single key can bypass the process.

TTL regression tests are in `contracts/stream/tests/governance_ttl.rs` and
cover:

- `Proposal(id)` survives a ledger advance past the persistent archival
  threshold thanks to the write-time bump.
- Reading a proposal re-extends the persistent TTL (`load_proposal` calls
  `bump_proposal`).
- `execute` succeeds after the full `GOVERNANCE_TIMELOCK_SECONDS` window
  because both `Proposal(id)` and `QuorumReachedAt(id)` are still on chain.
- A proposal with periodic reads can survive the full
  `MAX_PROPOSAL_AGE_SECONDS` window before `execute`.
- Negative control: executing a non-existent proposal id returns
  `ProposalNotFound`, which is the exact error surface a future bump-policy
  regression would expose.
- Drift guard: the local TTL constants match the contract's runtime
  constants via `timelock_seconds()` and `max_proposal_age_seconds()`.

## TTL and Timelock Relationship

### QuorumReachedAt Entry TTL
The `QuorumReachedAt(proposal_id)` persistent storage entry is bumped on:
- Every `approve` call when quorum is reached
- Every `execute` call when reading the entry

### ProposalApprovalIdx Entry TTL
The `ProposalApprovalIdx(proposal_id)` duplicate-approval index must outlive the
proposal record so quorum integrity cannot be undermined by storage expiry.
Its TTL is bumped on:
- Every `approve` call after recording an approval
- Every read via `get_approval_index` and `load_proposal`
- Every write via `save_approval_index`

Security implication: if the approval index archives before the proposal,
duplicate-approval detection silently fails and a signer could approve twice.
The coupling to `load_proposal` ensures any path that touches the proposal
also refreshes the index TTL when the index entry still exists.

### Constants
| Constant | Value | Duration |
|---|---|---|
| PERSISTENT_BUMP_AMOUNT | 120,960 ledgers | ~7 days |
| PERSISTENT_LIFETIME_THRESHOLD | 17,280 ledgers | ~1 day |
| GOVERNANCE_TIMELOCK_SECONDS | 172,800 seconds | 48 hours |

### Security
The bump amount (~7 days) comfortably exceeds the 48-hour timelock window,
ensuring QuorumReachedAt entries always outlive the timelock.

An expired or missing QuorumReachedAt entry causes execute to fail closed
with QuorumNotReached — the timelock is never silently re-opened.

## Property-Based Tests

`contracts/stream/tests/governance_proptest.rs` contains a proptest-driven
test suite that randomises signer-set sizes, approval orderings, and time
advances to assert core safety invariants that example-based tests can miss.

### Invariants

| # | Invariant | Assertion |
|---|-----------|-----------|
| 1 | **Below-quorum guard** | `execute` returns `QuorumNotReached` whenever `approvals < threshold`, no matter how much time has elapsed. |
| 2 | **Timelock guard** | `execute` returns `TimelockNotElapsed` for any `now < quorum_at + GOVERNANCE_TIMELOCK_SECONDS`. |
| 2b | **Timelock boundary (inclusive)** | `execute` succeeds at `now == quorum_at + GOVERNANCE_TIMELOCK_SECONDS` (strict `<` comparison). |
| 2c | **Post-boundary success** | `execute` succeeds for any `now > quorum_at + GOVERNANCE_TIMELOCK_SECONDS`. |
| 3 | **One-way executed flag** | A second `execute` on the same proposal always returns `AlreadyExecuted`. |
| 4 | **Full cross-product** | For all `(approval_count, time_delta)` pairs the outcome matches exactly the (quorum × timelock) truth table. |
| 5 | **Exactly-quorum boundary** | Exactly `threshold` approvals + `now >= exec_after` allows execution. |
| 5b | **One-below-quorum boundary** | Exactly `threshold - 1` approvals always blocks execution regardless of time. |

### Security notes

The highest-risk off-by-one locations are:

- **Timelock boundary**: `execute` uses `now < exec_after`, so the boundary is
  *inclusive* (`now == exec_after` should succeed). Properties 2 and 2b
  directly probe this.
- **Quorum boundary**: `approve` triggers quorum recording when
  `approval_count == threshold`. Properties 5 and 5b probe `threshold` and
  `threshold - 1` approvals explicitly.

### How to run

```bash
# Run the full proptest suite (256 cases per property, ~30 s):
cargo test --test governance_proptest --package fluxora_stream

# Run a single invariant:
cargo test --test governance_proptest prop_execute_fails_below_quorum

# Increase case count for deeper fuzzing:
PROPTEST_CASES=2000 cargo test --test governance_proptest
```

### Configuration

Each `proptest!` block uses `ProptestConfig::default()` with `cases: 256` and
a stable `source_file` annotation so that regression files in
`contracts/stream/proptest-regressions/governance_proptest.rs.txt` are
automatically replayed on every CI run.

To reproduce a specific failure, copy the failing seed from the test output
into `proptest-regressions/governance_proptest.rs.txt` or pass it via
`PROPTEST_SEED`.