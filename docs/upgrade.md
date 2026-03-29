# Fluxora Contract Upgrade Strategy

Version policy, migration runbook, and audit notes for operators, integrators, and auditors.

**Source of truth:** `contracts/stream/src/lib.rs` (`CONTRACT_VERSION` constant, `version()` entry-point)

---

## 1. CONTRACT_VERSION Policy

### What it is

`CONTRACT_VERSION` is a compile-time `u32` constant embedded in the WASM binary. It is returned by the permissionless `version()` entry-point with no storage access. Integrators call it to confirm which protocol revision is running before sending state-mutating transactions.

### Current value

```
CONTRACT_VERSION = 1
```

### When to increment

| Change type | Increment required? |
|---|---|
| Remove or rename a public entry-point | Yes |
| Change parameter type or order on any entry-point | Yes |
| Change a `ContractError` discriminant value | Yes |
| Change emitted event topic or payload shape | Yes |
| Change persistent storage key layout (breaks existing entries) | Yes |
| Add a new entry-point (purely additive) | Recommended (conservative) |
| Internal refactor — identical external behaviour | No |
| Documentation-only change | No |
| Gas optimisation — identical observable behaviour | No |
| Tighten validation (reject a previously-accepted edge case) | Document it; increment if integrators depend on the old behaviour |

### What counts as breaking

- Any change that causes a correctly-written v1 client to fail or misinterpret a response when talking to the new contract.
- Storage layout changes that make existing `Stream`, `Config`, or `RecipientStreams` entries unreadable after upgrade.
- Event shape changes that break indexers parsing `StreamCreated`, `Withdrawal`, `StreamEvent`, etc.

### What does NOT require an increment

- Adding new entry-points that old clients can safely ignore.
- Changing TTL bump constants (`INSTANCE_BUMP_AMOUNT`, `PERSISTENT_BUMP_AMOUNT`).
- Changing internal helper functions with no external surface.

---

## 2. version() Entry-Point Semantics

### Success semantics

- Returns `CONTRACT_VERSION` as a `u32`.
- No storage read, no token interaction, no auth check.
- Works before `init` is called (pre-flight deployment check).
- Idempotent: repeated calls always return the same value for a given deployment.

### Failure semantics

- Cannot fail. There are no error paths in `version()`.

### Authorization

- None. Any caller (wallet, indexer, script, another contract) may call `version()`.

### Gas

- Minimal. No storage access, no external calls.

---

## 3. Migration Runbook

Soroban contracts are **not upgradeable in-place** by default. A new `CONTRACT_VERSION` means deploying a new contract instance.

### Step-by-step

1. **Increment `CONTRACT_VERSION`** in `contracts/stream/src/lib.rs` before merging the breaking change.

2. **Build and deploy** the new WASM:
   ```bash
   cargo build --release -p fluxora_stream --target wasm32-unknown-unknown
   stellar contract deploy --wasm target/wasm32-unknown-unknown/release/fluxora_stream.wasm \
     --network mainnet --source $DEPLOYER_KEY
   ```

3. **Initialise** the new instance:
   ```bash
   stellar contract invoke --id $NEW_CONTRACT_ID -- init \
     --token $TOKEN_ADDRESS --admin $ADMIN_ADDRESS
   ```

4. **Verify version** before announcing migration:
   ```bash
   stellar contract invoke --id $NEW_CONTRACT_ID -- version
   # Must return the new CONTRACT_VERSION value
   ```

5. **Announce migration** with sufficient lead time (recommended: ≥ 14 days for mainnet) so that:
   - Recipients can withdraw accrued funds from the old instance.
   - Senders can cancel and recreate streams on the new instance if desired.
   - Indexers and wallets can update their `CONTRACT_ID` references.

6. **Update all integrations** to point at the new `CONTRACT_ID`. Integrations should assert:
   ```text
   assert version() == EXPECTED_VERSION
   ```
   before sending any state-mutating transaction.

7. **Do not destroy the old instance** until all active streams have been settled (withdrawn or cancelled). Persistent storage entries on the old instance remain readable as long as the instance exists and its TTL has not expired.

### Stream migration

There is no on-chain migration path for stream state between contract versions. Options:

| Stream status | Recommended action |
|---|---|
| Active | Let it run to completion on the old instance, or sender cancels and recreates on new instance |
| Paused | Sender resumes, then withdraws or cancels on old instance |
| Cancelled | Recipient withdraws frozen accrued amount on old instance |
| Completed | Recipient withdraws remaining amount on old instance; optionally close via `close_completed_stream` |

---

## 4. Integrator Checklist

Before interacting with any Fluxora contract instance:

- [ ] Call `version()` and assert it equals the version your client was built against.
- [ ] Call `get_config()` to confirm the token address matches the expected asset.
- [ ] Confirm the `CONTRACT_ID` matches the announced deployment.
- [ ] Subscribe to `StreamCreated` events using the new `CONTRACT_ID` (not the old one).

---

## 5. Residual Risks and Audit Notes

1. **No on-chain enforcement of increment discipline.** If a developer deploys a breaking change without incrementing `CONTRACT_VERSION`, integrators will not detect the incompatibility until a runtime failure occurs. Mitigation: CI check that fails if `CONTRACT_VERSION` is unchanged on a PR that modifies public entry-points, event types, or error codes.

2. **TTL expiry.** Persistent stream entries have a finite TTL. If an old contract instance is abandoned without being bumped, stream entries may expire before recipients withdraw. Operators must ensure recipients are notified well before TTL expiry.

3. **No upgrade path for in-flight streams.** Streams created on v1 cannot be migrated to v2 on-chain. This is a deliberate design choice (simplicity, auditability) but means migration windows must be long enough for all streams to settle.

4. **Admin key continuity.** The admin address is set at `init` time and is immutable via `init`. Use `set_admin` to rotate the admin key before migrating to a new instance, and call `init` on the new instance with the new admin address.

5. **Token address immutability.** The token is fixed at `init` time. A new contract version that needs a different token requires a new `init` call with the new token address — existing streams on the old instance are unaffected.
