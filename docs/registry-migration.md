# Contract Registry Migration

## Overview

The Fluxora protocol supports **registry migration**: swapping the underlying
`FluxoraStream` contract that a `FluxoraFactory` instance points to.  This
allows the protocol to deploy a new stream contract version and have the
factory route new stream creations to the new instance while existing streams
continue running on the old instance.

Two entrypoints can trigger a registry migration:

| Entrypoint | Authorizer | Timelock | Location |
|---|---|---|---|
| `FluxoraFactory::set_stream_contract` | Factory admin | None | `contracts/factory/src/lib.rs` |
| `CallData::FactorySetStreamContract` | Governance quorum + 48h | Yes | `contracts/governance/src/lib.rs` |

---

## Migration Paths

### 1. Direct Admin: `FluxoraFactory::set_stream_contract`

```
Admin → require_admin() → validate_stream_contract() → persist → bump TTL → emit stm_upd
```

The factory admin calls `set_stream_contract(new_address)` directly on the
factory contract.  The call:

1. Reads the currently stored `StreamContract` address (`DataKey` discriminant
   1).  Returns `FactoryError::NotInitialized` if the factory has not been
   initialized.

2. **Same-address no-op** — if `new_address == old_address`, returns `Ok(())`
   immediately.  No validation, no storage write, no event emission, no TTL
   bump.  This prevents misleading `stm_upd` events when callers
   inadvertently submit the current address.

3. Calls `require_admin()` — the stored admin must authorize via
   `require_auth()`.  Non-admin callers are rejected at the host level (panic
   / failed auth).

4. Calls `validate_stream_contract()` — invokes `FluxoraStream::version()` on
   the candidate via `try_version`.  If the candidate does not expose a
   `version()` entrypoint (EOA, non-contract address, or contract without the
   `FluxoraStream` interface), returns
   `FactoryError::InvalidStreamContract`.  The previously configured
   `stream_contract` is left untouched.

5. Persists the new address to `DataKey::StreamContract`.

6. Calls `bump_registry_ttl()` — extends the persistent TTL on
   `DataKey::FactoryStreamIds` (the stream ID registry) so that existing
   entries created under the old contract remain queryable through the new
   contract without depending on active writes.  No-op if the registry is
   empty.

7. Calls `bump_instance()` — extends instance storage TTL.

8. Emits `StreamContractUpdated { old_contract, new_contract }` with topic
   `stm_upd`.

### 2. Governance-Governed: `CallData::FactorySetStreamContract`

```
Signer propose → Signers approve → QuorumReached → Timelock (48h) → Execute → dispatch_call → env.invoke_contract("set_stream_contract")
```

The governance contract's `CallData::FactorySetStreamContract(Address)`
variant allows a multi-sig proposal to execute `set_stream_contract` on a
target factory address after accumulating quorum and waiting through the 48h
timelock.  The target factory runs the exact same `set_stream_contract` logic
as the direct admin path (steps 1–8 above), including same-address no-op and
registry TTL bump.

The governance dispatch (`dispatch_call` at
`contracts/governance/src/lib.rs:372`) invokes the target contract via
`env.invoke_contract::<()>(target, "set_stream_contract", (new_contract,))`.
If the target is not a `FluxoraFactory` (or does not implement
`set_stream_contract`), the cross-contract call traps and the entire
governance proposal execution reverts — including the `executed = true` flag
(CEI: state mutation precedes interaction).

#### Authorization comparison

| Aspect | Direct admin | Governance |
|---|---|---|
| Auth | Single admin key | N-of-M multi-sig + 48h timelock |
| Timelock | None | `GOVERNANCE_TIMELOCK_SECONDS` (172,800s / 48h) |
| Audit trail | Single event | Proposal lifecycle + event |
| Key compromise risk | High (single key) | Low (threshold + delay) |

---

## Storage Layout

### Instance storage (factory)

| `DataKey` | Discriminant | Type | Set by | Purpose |
|---|---|---|---|---|
| `Admin` | 0 | `Address` | `init`, `set_admin` | Factory admin |
| `StreamContract` | 1 | `Address` | `init`, `set_stream_contract` | Target stream contract |

`StreamContract` is loaded by `load_policy()` on every `create_stream` /
`create_streams` call.  After migration the new address is used immediately
for subsequent stream creations.

### Persistent storage (factory)

| `DataKey` | Discriminant | Type | Set by | Purpose |
|---|---|---|---|---|
| `FactoryStreamIds` | 6 | `Vec<u64>` | `create_stream`, `create_streams` | Registry of all stream IDs created through this factory |

The registry is write-only-append.  Entries are never removed.  The TTL is
bumped on:
- Every `append_stream_id` (single `create_stream`) 
- Every `append_stream_ids_batch` (batch `create_streams`)
- Every `set_stream_contract` migration (via `bump_registry_ttl`, only when
  the registry is non-empty)

The registry is **not** TTL-bumped on read-only views
(`get_factory_streams_paginated`, `get_factory_stream_count`).  A factory
that has no new stream creations for longer than `PERSISTENT_BUMP_AMOUNT`
ledgers (~7 days at 5s/ledger) and no admin operations that call
`bump_registry_ttl` may see its registry expire.  Indexers and off-chain
tooling SHOULD keep the registry alive by creating a no-op stream or by
performing periodic admin reads that trigger instance TTL extensions (the
instance itself is extended by `bump_instance` on every admin operation).

### TTL constants

| Constant | Value | Used for |
|---|---|---|
| `INSTANCE_LIFETIME_THRESHOLD` | 17,280 ledgers (~24h) | Instance entry extension trigger |
| `INSTANCE_BUMP_AMOUNT` | 120,960 ledgers (~7d) | Instance entry extension target |
| `PERSISTENT_LIFETIME_THRESHOLD` | 17,280 ledgers (~24h) | Persistent entry extension trigger |
| `PERSISTENT_BUMP_AMOUNT` | 120,960 ledgers (~7d) | Persistent entry extension target |

---

## Edge Cases and Their Handling

### E1. Same-address migration

**Behavior**: Calling `set_stream_contract(current_address)` returns
`Ok(())` without side effects — no validation, no storage write, no event,
no TTL bump.

**Rationale**: Prevent misleading events and unnecessary state changes when
callers (including governance proposals) inadvertently submit the current
address.

**Regression surface**: Indexers that previously relied on `stm_upd` events
for every `set_stream_contract` call will no longer see events for
no-op calls.  This is by design — the event only fires when the address
actually changes.

### E2. Migration while streams are active

**Behavior**: Existing streams on the old contract continue running
independently.  The factory registry retains all previously created stream
IDs.  New `create_stream`/`create_streams` calls route to the new contract.

**Rationale**: Streams are independent contracts with their own storage and
TTL.  The factory does not own or manage streams after creation — it only
records their IDs.

**Regression surface**: None.  Existing behavior is unchanged.  Tests in
`test_registry_persists_across_stream_contract_migration` verify that
registry entries survive migration.

### E3. Invalid stream contract address

**Behavior**: An EOA, a non-contract address, or a contract that does not
implement `FluxoraStream::version()` returns
`FactoryError::InvalidStreamContract`.  The old `stream_contract` is
preserved.

**Rationale**: Prevent silent misconfiguration that would trap on the next
`create_stream` call.

**Regression surface**: None.  This validation existed before hardening.

### E4. Uninitialized factory

**Behavior**: Calling `set_stream_contract` before `init` returns
`FactoryError::NotInitialized`.

**Rationale**: Without a stored `StreamContract` the factory cannot
determine the "old" address.  The no-op check cannot run because there is
no current address to compare against.

**Regression surface**: None.  Previously returned `NotInitialized` via
`require_admin` which reads `DataKey::Admin`.  Now returns via
`DataKey::StreamContract` read — same error, different implementation
path.

### E5. Registry expiry after migration

**Behavior**: The `bump_registry_ttl` helper extends the registry TTL
during `set_stream_contract`.  If the factory receives no new writes
post-migration, the registry may eventually expire (after
`PERSISTENT_BUMP_AMOUNT` ledgers).

**Rationale**: TTL extension on reads would add gas cost to every
`get_factory_streams_paginated` call.  The one-time bump at migration time
provides a ~7-day window.  Indexers that need longer retention should
periodically extend via a no-op stream creation.

**Regression surface**: New behavior — registry TTL was not previously
bumped during migration.  The change is additive (no existing behavior
removed).

### E6. Governance proposal targets non-factory contract

**Behavior**: A `FactorySetStreamContract` proposal whose `target` is not
a `FluxoraFactory` (or does not expose `set_stream_contract`) will revert
during `execute()`.  The proposal's `executed` flag is NOT set (CEI: state
mutation precedes interaction, but the interaction traps and the entire
transaction reverts, including the `executed = true` write).

**Rationale**: Standard Soroban revert-on-trap behavior.  The proposal can
be re-submitted with the correct target address.

**Regression surface**: None.  Existing behavior is unchanged.

### E7. Governance `FactorySetStreamContract` with current address

**Behavior**: Same as E1 — the governance proposal executes successfully
(no revert), the factory's `set_stream_contract` returns `Ok(())` as a
no-op, and no `stm_upd` event is emitted.  The proposal is marked executed
(because governance `execute()` does not distinguish between no-op and
actual dispatch outcomes from the factory).

**Rationale**: The governance proposal lifecycle completes normally.  The
no-op happens at the factory level, not the governance level.  This is
consistent with other `CallData` variants that succeed without side
effects (e.g. `Noop`).

**Regression surface**: Previously, a `FactorySetStreamContract` proposal
with the current address would emit a misleading `stm_upd` event.  After
hardening no event is emitted.  Indexers should not rely on `stm_upd`
being present for every executed `FactorySetStreamContract` proposal.

---

## Gas Profile

The table below lists the gas (instruction budget) for each `set_stream_contract`
call path.  Values are approximate and depend on the Soroban host environment.

| Operation | Budget estimate | Notes |
|---|---|---|
| Same-address no-op | ~1,000 instructions | Storage read + comparison; no cross-contract call |
| Valid migration (admin) | ~10,000 instructions | Auth + cross-contract version check + 2 storage writes + 2 TTL extends + event |
| Valid migration (governance) | ~35,000 instructions | Above + proposal load + approval filter + event; CEI pattern |

The `validate_stream_contract` cross-contract call to `version()` is the
primary cost driver for a real migration.  `version()` is intentionally
storage-free and always succeeds, so the cross-contract cost is
predictable and low.

---

## Upgrade Compatibility

### Storage compatibility

`set_stream_contract` only writes `DataKey::StreamContract` (discriminant 1,
instance storage).  This discriminant is frozen and append-only per the
`DataKey` evolution policy documented in `docs/storage.md`.  Adding new
fields to `FluxoraStream`'s struct or new `DataKey` variants to the stream
contract does not affect this write path.

### ABI compatibility

The function signature `set_stream_contract(Address) → Result<(), FactoryError>`
is frozen per `docs/ABI_STABILITY.md`.  Adding new governed operations to
`CallData` requires only appending a new variant (append-only).

### Existing stream compatibility

Streams created under the old `stream_contract` continue to operate
independently after migration:

- Active streams: accrue, withdraw, cancel, pause, resume — all work on the
  old contract instance.
- Completed streams: withdrawal works on the old instance.
- Cancelled streams: withdrawal works on the old instance.

The factory does not hold any stream state beyond the ID list.  Streams are
independent Soroban contract instances with their own storage, admin, and TTL.

---

## Regression Surface

The following behaviors are locked by the existing test suite and must not
change:

| # | Behavior | Test(s) | File |
|---|---|---|---|
| R1 | Valid migration: address changes, `stm_upd` emitted with old/new | `test_set_stream_contract_valid_migration_succeeds` | `contracts/factory/src/lib.rs` |
| R2 | Same-address: no event, no state change | `test_set_stream_contract_same_address_noop` | `contracts/factory/src/lib.rs` |
| R3 | Pre-init: returns `NotInitialized` | `test_set_stream_contract_before_init_errors` | `contracts/factory/src/lib.rs` |
| R4 | EOA: returns `InvalidStreamContract`, old preserved | `test_set_stream_contract_rejects_eoa` | `contracts/factory/src/lib.rs` |
| R5 | Wrong interface: returns `InvalidStreamContract`, old preserved | `test_set_stream_contract_rejects_non_fluxora_stream` | `contracts/factory/src/lib.rs` |
| R6 | Registry persists after migration | `test_registry_persists_across_stream_contract_migration` | `contracts/factory/src/lib.rs` |
| R7 | Governance dispatches `FactorySetStreamContract` correctly | `test_factory_set_stream_contract_dispatches_via_governance` | `contracts/governance/src/lib.rs` |
| R8 | `stm_upd` event shape: `StreamContractUpdated { old_contract, new_contract }` | `test_set_stream_contract_valid_migration_succeeds` | `contracts/factory/src/lib.rs` |
| R9 | Registry TTL bumped after migration | Implicit in R6 (registry entries queryable post-migration) | `contracts/factory/src/lib.rs` |
| R10 | `validate_stream_contract` rejects EOA in `init` | `test_init_rejects_eoa` (in `factory_init_security.rs`) | `contracts/factory/tests/factory_init_security.rs` |
| R11 | Calldata round-trip: `FactorySetStreamContract` XDR encodes/decodes | `test_calldata_variants_roundtrip` | `contracts/governance/src/lib.rs` |
| R12 | Shape validation: raw bytes / arbitrary structs rejected as calldata | `test_calldata_shape_validation_disallowed_target_functions_rejected` | `contracts/governance/src/lib.rs` |

### Non-goals (explicitly out of scope)

The following are NOT covered by registry migration tests and are handled
by separate subsystems:

- Stream contract `upgrade()` entrypoint (in-place WASM replacement) — see
  `docs/upgrade.md`
- Factory batch-cap enforcement — see `docs/factory.md`
- Governance threshold changes — see `docs/governance.md`
- TTL expiry of individual stream records — see `docs/storage.md`
- Disabled integration tests (gas regression, signer-index proptest,
  factory setters, factory e2e, adversarial auth) — see Cargo.toml
  `test = false` entries

---

## Test Coverage

### Factory inline tests (`contracts/factory/src/lib.rs`, `mod tests`)

| Test | Coverage |
|---|---|
| `test_set_stream_contract_same_address_noop` | E1 — same-address no-op |
| `test_set_stream_contract_valid_migration_succeeds` | Happy path + R1, R8 |
| `test_set_stream_contract_before_init_errors` | E4 + R3 |
| `test_set_stream_contract_rejects_eoa` | E3 + R4 |
| `test_set_stream_contract_rejects_non_fluxora_stream` | E3 + R5 |
| `test_registry_persists_across_stream_contract_migration` | E2 + R6, R9 |

### Governance inline tests (`contracts/governance/src/lib.rs`, `mod tests`)

| Test | Coverage |
|---|---|
| `test_factory_set_stream_contract_dispatches_via_governance` | Path 2 + R7 |
| `test_governance_registry_migration_non_factory_target_reverts` | E6 — non-factory target reverts |
| `test_governance_registry_migration_same_address` | E7 — same-address no-op via governance |
| `test_governance_registry_migration_emits_correct_events` | Event verification for governance dispatch |
| `test_calldata_variants_roundtrip` | R11 |
| `test_calldata_shape_validation_disallowed_target_functions_rejected` | R12 |
| `test_calldata_shape_validation_no_selector_collision_bypass` | R12 |

---

## References

- `contracts/factory/src/lib.rs` — `set_stream_contract`, `bump_registry_ttl`,
  `validate_stream_contract`, `load_policy`
- `contracts/governance/src/lib.rs` — `dispatch_call` (FactorySetStreamContract
  arm), `CallData::FactorySetStreamContract`
- `docs/factory.md` — Factory policy documentation
- `docs/governance.md` — Governance contract documentation
- `docs/upgrade.md` — Upgrade and version policy
- `docs/storage.md` — Storage layout and `DataKey` evolution policy
- `docs/ABI_STABILITY.md` — ABI stability guarantees and breaking change
  classification
