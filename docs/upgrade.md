# Fluxora Contract Upgrade Strategy

Version policy, migration runbook, and audit notes for operators, integrators, and auditors.

**Source of truth:** `contracts/stream/src/lib.rs` (`CONTRACT_VERSION` constant, `version()` entry-point)
**ABI stability rules:** [`docs/ABI_STABILITY.md`](./ABI_STABILITY.md) — canonical definition of what counts as a breaking change for entrypoints, error codes, event schemas, and storage discriminants.

---

## 1. CONTRACT_VERSION Policy

### What it is

`CONTRACT_VERSION` is a compile-time `u32` constant embedded in the WASM binary. It is returned by the permissionless `version()` entry-point with no storage access. Integrators call it to confirm which protocol revision is running before sending state-mutating transactions.

### Current value

```
CONTRACT_VERSION = 9
```

### Version history

| Version | Change summary |
|---|---|
| 1 | Initial release |
| 2 | `Stream` struct gained `checkpointed_amount: i128` and `checkpointed_at: u64` for safe rate-decrease support |
| 3 | `Stream` struct gained `memo: Option<Bytes>`; `StreamCreated` event gained `memo` field; `DataKey::StreamMemo(u64)` added at discriminant 10; `create_stream`/`create_streams` gained `memo` parameter; `get_stream_memo` entry-point added |
| 4 | `TotalLiabilities` instance key for escrow accounting; accrual paths track last observed ledger timestamp for clock-regression detection |
| 5 | `withdraw_dust_threshold: i128` added to `Stream` struct and creation params; `DataKey::PausedStreamCount` added and maintained across pause/resume/cancel/complete transitions; `get_paused_stream_count()` O(1) view added |
| 6 | Sweep excess authorization update; added additive `DataKey` variants 15–28 (`WithdrawNonce`, `PauseState`, `ReentrancyLock`, `RecipientStreamPage`, `RecipientStreamPageCount`, `PendingRecipientUpdate`, `IdReservation`, `MaxRatePerSecond`, `DelegatedWithdrawNonce`, `LastPauseRecord`, `RotationHistory`, `LastAccrualLedgerTimestamp`, `PausedStreamCount`, `TotalKeeperFeesPaid`) |
| 7 | `Stream` and `CreateStreamParams` gained optional `witness: Option<Address>` for off-chain compliance attestation cancellation (`witnessed_cancel_stream` entry-point added); `DataKey::SenderStreams(Address)` at discriminant 29, `DataKey::AutoRenewEnabled(u64)` at discriminant 30 for auto-renewal; `DataKey::PendingStreamOffer(u64)` at discriminant 31 and `DataKey::RecipientPendingOffers(Address)` at discriminant 32 for two-phase offer-then-accept stream creation; `create_stream_offer`, `accept_stream_offer`, `reject_stream_offer`, `cancel_stream_offer`, `get_stream_offer`, `get_recipient_pending_offers` entrypoints added; new `ContractError` variants `OfferNotFound` (37), `OfferExpired` (38), `OfferWrongRecipient` (39), `OfferWrongSender` (40); `Stream` and `CreateStreamParams` gained optional `irrevocable: Option<bool>` field blocking all cancel/shorten paths |

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

For the exhaustive, category-by-category breakdown see **[`docs/ABI_STABILITY.md §3`](./ABI_STABILITY.md#3-what-counts-as-a-breaking-change)**.

### What does NOT require an increment

- Adding new entry-points that old clients can safely ignore.
- Changing TTL bump constants (`INSTANCE_BUMP_AMOUNT`, `PERSISTENT_BUMP_AMOUNT`).
- Changing internal helper functions with no external surface.
- Appending new `DataKey` storage variants (append-only; existing entries stay
  byte-identical and readable). See the version-specific rationale in
  `contracts/stream/src/checksum.rs`'s module doc-comment for why some additive
  `DataKey` growth did not require an immediate version bump.

> **Note (transfer_sender):** The `transfer_sender` entry-point is a purely additive
> new entry-point. Old clients that do not call it are unaffected. `CONTRACT_VERSION`
> was incremented conservatively per the policy above. Indexers should subscribe to
> the new `sndr_xfr` event to track sender rotations.

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

### Paused-stream counter backfill caveat (v5)

`CONTRACT_VERSION = 5` introduces an instance key, `DataKey::PausedStreamCount`, used by
`get_paused_stream_count()` to expose an O(1) protocol-wide gauge of how many streams are
currently paused.

Because existing deployments do **not** have historical pause state materialized in this new
instance key, an upgraded instance starts with the counter effectively unset / `0`.

Operational consequences:
- Fresh deployments initialised on v7 track the exact live paused-stream count immediately.
- Upgraded deployments report `0` until post-upgrade stream transitions repopulate the counter.
- Legacy streams that were already paused before the upgrade are **not** backfilled.
- The first post-upgrade `resume_*`, `cancel_*`, or terminal completion of such a legacy paused
  stream will **not** make the counter go negative; the implementation uses saturating decrement
  semantics for safety.
- Once a legacy stream experiences a post-upgrade pause transition, it is tracked normally again.

If an exact paused-stream count is required immediately after upgrading an already-live instance,
operators must reconstruct it off-chain once (for example, by enumerating stream state) and treat
`get_paused_stream_count()` as authoritative only for post-upgrade transitions thereafter.

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

6. **Machine-checked `CONTRACT_VERSION` vs `DataKey` variant count cross-check.** To prevent version drift when new storage keys are appended, `contracts/stream/tests/storage_key_compat.rs` enforces a machine-checked mapping between `CONTRACT_VERSION` and expected `DataKey` variant count (currently **36** for `CONTRACT_VERSION = 9`). Whenever a new `DataKey` variant is appended or `CONTRACT_VERSION` is incremented, developers MUST update:
   - `expected_datakey_count_for_version()` and `all_live_datakey_variants()` in `contracts/stream/tests/storage_key_compat.rs`
   - Discriminant tables & variant count tests in `contracts/stream/src/checksum.rs`
   - Version history & policy table in `docs/upgrade.md`

---

## 6. CI Toolchain Verification

The `.github/workflows/ci.yml` workflow includes a step in all Rust-related jobs (`lint`, `build`, `test`, `coverage`) that verifies the `rustc` version in the environment matches the version pinned in the `rust-toolchain.toml` file.

This is a safety net to prevent "toolchain drift", where a change in the CI environment (e.g., an update to the `dtolnay/rust-toolchain@stable` action) could cause the contract to be built or tested with a different compiler version than is specified in the repository.

The verification is performed by the `script/verify_rust_version.py` script. If a mismatch is detected, the script prints an error and exits with a non-zero status code, failing the CI job. This ensures that all builds and tests are performed with the intended, pinned toolchain. This check is independent of, and a safety net for, any future change to which GitHub Action resolves the toolchain.

### MSRV Cross-Check

To ensure `cargo` enforces the Minimum Supported Rust Version (MSRV) on every invocation (including local developer builds), each crate's `Cargo.toml` (`contracts/stream/Cargo.toml`, `contracts/factory/Cargo.toml`, and `contracts/governance/Cargo.toml`) explicitly declares a `rust-version` field. 

The `tests/test_rust_toolchain_pin.py` test suite asserts that the `rust-version` in each of these `Cargo.toml` manifests matches the pinned channel in `rust-toolchain.toml`, ensuring the MSRV is synchronized across the entire repository.

---

## 7. Paginated Export Views (Issue #429)

Bounded, paginated view entrypoints support off-chain export and migration between contract instances without unbounded loops or memory usage.

### Motivation

Operators need to export stream data for:
- Migration between contract versions (no on-chain upgrade path exists)
- Off-chain analytics and reporting
- Backup and audit trails
- Integration with external systems

Without pagination, `get_recipient_streams` returns **all** streams unbounded, which can exhaust memory or hit gas limits with large portfolios.

### Entrypoints

#### `get_streams_by_id_range(start_id, end_id, limit) -> Vec<Stream>`

Returns streams within an ID range `[start_id, end_id]` with a strict result limit.

**Parameters:**
- `start_id: u64` — First stream ID to include (inclusive)
- `end_id: u64` — Last stream ID to include (inclusive). Use `u64::MAX` for open-ended.
- `limit: u64` — Maximum streams to return (capped at `MAX_PAGE_SIZE = 100`)

**Returns:**
- `Vec<Stream>` — Stream structs in ascending ID order
- Empty vector if `start_id > end_id` or no streams exist in range
- Closed/archived stream IDs (holes) are silently skipped

**Edge Case Behavior:**
- **Holey Ranges:** Ranges containing closed or never-created IDs (holes) skip missing IDs and return only active streams in the specified range.
- **Limit Clamping:** Limits above `MAX_PAGE_SIZE = 100` are strictly clamped to `MAX_PAGE_SIZE`.
- **Start Beyond Max:** If `start_id` is greater than the highest created ID (e.g. `get_stream_count()`), the call returns an empty vector.
- **Zero Limit:** Requesting a page with `limit = 0` returns an empty vector.

**DoS Protection:**
- `limit` is capped at `MAX_PAGE_SIZE` (100) regardless of input. This bounds the maximum execution and read costs, preventing memory or gas exhaustion DoS vectors.
- Gas cost is O(min(limit, actual_results)), not O(range_size)
- Each stream lookup is O(1)

**Migration Pattern:**
```rust
let total = client.get_stream_count();
let mut start = 0u64;
while start < total {
    let page = client.get_streams_by_id_range(&start, &(start + 99), &100);
    // Export page...
    start += 100;
}
```

#### `get_recipient_streams_paginated(recipient, cursor, limit) -> Vec<u64>`

Cursor-based pagination for recipient stream export.

**Parameters:**
- `recipient: Address` — Address to query
- `cursor: u64` — 0-based starting index in the recipient's stream list
- `limit: u64` — Maximum streams to return (capped at `MAX_PAGE_SIZE = 100`)

**Returns:**
- `Vec<u64>` — Stream IDs in ascending order
- Empty vector if `cursor >= recipient_stream_count`

**Cursor Semantics:**
- Cursor is a 0-based index into the sorted recipient stream list
- After each call: `next_cursor = cursor + result.len()`
- When `result.len() < limit`, you've reached the end
- List mutations (insertions/removals) shift indices naturally

**DoS Protection:**
- `limit` is capped at `MAX_PAGE_SIZE` (100)
- Only loads the requested page, not the entire recipient list
- Gas cost is O(limit), not O(total_recipient_streams)

**Full Export Pattern:**
```rust
let mut cursor = 0u64;
loop {
    let page = client.get_recipient_streams_paginated(&recipient, &cursor, &50);
    if page.is_empty() { break; }
    // Export page...
    cursor += page.len() as u64;
}
```

### Safety Limits

| Constant | Value | Purpose |
|----------|-------|---------|
| `MAX_PAGE_SIZE` | 100 | Maximum results per paginated query |

These limits prevent:
- Memory exhaustion from unbounded vector returns
- Gas limit violations from excessive storage reads
- DoS via intentionally large limit parameters

### Comparison: Old vs New

| Scenario | Old Approach | New Approach |
|----------|--------------|--------------|
| Export 1000 streams | `get_recipient_streams` → unbounded, may fail | `get_streams_by_id_range` with pagination → reliable |
| Large portfolio query | Risk of gas/memory exhaustion | Bounded pages, predictable gas |
| Migration script | Complex retry logic | Simple cursor iteration |

### Testing Requirements

All paginated views are tested for:
- ✅ Basic pagination (correct items, order)
- ✅ Empty ranges/cursors return empty
- ✅ `MAX_PAGE_SIZE` enforcement (requests > 100 capped)
- ✅ Closed stream handling (gracefully skipped)
- ✅ Open-ended ranges (`u64::MAX`)
- ✅ Zero limit returns empty
- ✅ Cursor beyond end returns empty
- ✅ Multiple recipient isolation
- ✅ Full export workflow (accumulate all pages)

See `contracts/stream/src/test.rs` for the complete test suite.



# On-Chain Contract Upgrades

## Overview

Fluxora supports in-place contract upgrades via the `upgrade()` entrypoint. This allows the protocol to ship security fixes and feature improvements without requiring integrators to migrate to a new contract address.

## How It Works

The `upgrade()` function calls Soroban's `update_current_contract_wasm` host function, which atomically replaces the contract's WASM code in-place.

## Authorization

Upgrades are **admin-only**:

- The admin address (set during `init()`) must sign the transaction
- In production, the admin should be the governance contract (`fluxora_governance`)
- Governance requires multi-signer approval (quorum) before the upgrade can execute

## Storage Compatibility

The upgraded WASM must maintain backward-compatible storage layout.

### Safe Changes
- Add new fields to `Stream` struct (append at end)
- Add new variants to `DataKey` enum (append at end)
- Add new functions to the contract
- Gas optimizations

### Unsafe Changes
- Reorder `Stream` struct fields
- Remove `Stream` struct fields
- Reorder `DataKey` enum variants
- Remove `DataKey` variants

## Upgrade Workflow

### 1. Build New WASM
```bash
cargo build --target wasm32-unknown-unknown --release -p fluxora_stream
stellar contract optimize --wasm target/wasm32-unknown-unknown/release/fluxora_stream.wasm
```

---

## 8. Cargo.lock Determinism

### Why it matters

Every WASM binary Fluxora ships must be byte-for-byte reproducible so the
checksum recorded in `wasm/checksums.sha256` can be independently verified.
The build depends on a fixed set of compiled dependencies; if any dependency
version changes between two builds the output WASM changes, invalidating the
reference checksum and breaking `script/verify-wasm-checksum.sh`.

`Cargo.lock` is the mechanism that pins every transitive dependency to an
exact version. It must be:

- **Committed** — present in version control so every `git clone` + build
  uses the identical dependency set.
- **Unchanged** — not silently updated by a `cargo update` run that no one
  reviewed, for example when an unpinned `^` version specifier resolves to a
  newer compatible release.

This is the "Dependency resolution" residual risk documented in
`contracts/stream/src/checksum.rs`.

### CI enforcement

The `build` job in `.github/workflows/ci.yml` runs the following step
**before** any WASM build step:

```yaml
- name: Verify Cargo.lock is committed and unchanged
  run: |
    set -euo pipefail
    if ! cargo update --locked --workspace; then
      echo "::error::Cargo.lock would change on dependency resolution ..."
      echo "::error::This breaks the WASM build-reproducibility contract documented
            in contracts/stream/src/checksum.rs ('Dependency resolution: Cargo.lock
            must be committed and unchanged')."
      echo "::error::If this dependency change is intentional, run 'cargo update'
            locally, review the diff, and commit the updated Cargo.lock alongside
            your change."
      exit 1
    fi
```

`cargo update --locked` exits non-zero if satisfying the current `Cargo.toml`
manifests would require any change to `Cargo.lock`. The step fails the entire
`build` job before any WASM binary is produced, so no artifact with an
unverified checksum can be uploaded.

### Structural test suite

`contracts/stream/tests/cargo_lock_determinism.rs` provides a complementary
layer of always-on structural checks that run during the normal `cargo test`
flow (no special environment required). The eight tests it contains are:

| Test | What it checks |
|------|----------------|
| `cargo_lock_exists_at_workspace_root` | `Cargo.lock` is present at the workspace root |
| `cargo_lock_is_non_empty_and_contains_packages` | The lockfile is not a stub — it contains at least one `[[package]]` entry |
| `cargo_lock_records_soroban_sdk` | `soroban-sdk` is recorded in the lockfile |
| `stream_cargo_toml_soroban_sdk_is_exact_pin` | The `soroban-sdk` version in `contracts/stream/Cargo.toml` is an exact pin with no `^`, `~`, `*`, or `>=` range specifier |
| `workspace_cargo_toml_has_no_patch_table` | The workspace `Cargo.toml` contains no `[patch]` table |
| `workspace_cargo_toml_uses_resolver_v2` | The workspace `Cargo.toml` declares `resolver = "2"` |
| `cargo_lock_has_modern_format_version` | The lockfile declares format version 3 or 4 (generated by Cargo ≥ 1.78) |
| `cargo_lock_soroban_sdk_version_matches_cargo_toml_pin` | The `soroban-sdk` version in `Cargo.lock` matches the exact pin in `contracts/stream/Cargo.toml` |

These tests provide an earlier, local signal for the same class of violations
the CI gate catches, and they produce diagnostic messages that point directly
at this section and at `contracts/stream/src/checksum.rs`.

### Security assumptions

- **Exact version pins.** Every dependency that influences WASM output must be
  pinned with an exact version string (e.g. `"21.7.7"`), not a caret range
  (`"^21.7.7"`). A caret range allows `cargo update` to silently resolve to a
  newer compatible release, changing the WASM without any diff in source code.
- **No `[patch]` overrides.** A `[patch]` entry in the workspace `Cargo.toml`
  can substitute a local path or git revision that differs between machines.
  The resulting WASM may differ between environments, and the lockfile does not
  capture the substituted source completely enough for `--locked` to enforce
  byte-for-byte reproducibility.
- **Resolver version 2.** Without `resolver = "2"`, Cargo can unify features
  differently across workspace members, potentially activating features in the
  WASM build target that alter the compiled output.
- **Modern lockfile format.** A format version 1 or 2 lockfile (produced by
  Cargo < 1.38) would be silently upgraded on the next `cargo` invocation,
  making `Cargo.lock` appear modified even though no dependency versions
  changed. This causes spurious CI failures and masks real drift.

### Recovery procedure

When CI fails the "Verify Cargo.lock is committed and unchanged" step, or
when the `cargo_lock_determinism` tests fail locally:

1. **Identify the drift.** Run `cargo update --workspace` locally and inspect
   `git diff Cargo.lock`. Every changed `[[package]]` entry represents a
   dependency version change that will alter the WASM binary.

2. **Decide intentionality.** If the change is a deliberate upgrade (e.g. a
   security patch in a transitive dependency), proceed to step 3. If it is
   unintentional (e.g. a caret range resolving to a new patch release without
   review), pin the drifting dependency explicitly in the relevant `Cargo.toml`
   and run `cargo update --workspace` again until the diff is empty.

3. **Regenerate the reference checksum.** Because the WASM binary will change,
   the checksum in `wasm/checksums.sha256` is now invalid. Regenerate it:

   ```bash
   cargo build --release -p fluxora_stream --target wasm32-unknown-unknown
   bash script/update-wasm-checksums.sh
   ```

4. **Commit atomically.** Commit `Cargo.lock`, `wasm/checksums.sha256`, and
   any `Cargo.toml` pin changes together in a single commit so the three files
   remain consistent:

   ```bash
   git add Cargo.lock wasm/checksums.sha256 contracts/stream/Cargo.toml
   git commit -m "chore: update Cargo.lock and regenerate WASM checksum after dependency bump"
   ```

5. **Verify.** Run `cargo build --locked -p fluxora_stream` locally to confirm
   `cargo build --locked` passes before pushing.

### Relationship to other reproducibility invariants

The Cargo.lock determinism guarantee is one of five invariants that together
make a build reproducible. The full list is documented in
`contracts/stream/src/checksum.rs` §"Build reproducibility contract":

| Invariant | Enforcement |
|-----------|-------------|
| Rust toolchain pinned | `rust-toolchain.toml`; verified by `script/verify_rust_version.py` in every CI job |
| `soroban-sdk` version pinned | `contracts/stream/Cargo.toml` exact pin; checked by `cargo_lock_determinism` tests |
| Build profile is `--release` + `wasm32-unknown-unknown` | CI `build` job `cargo build` invocation |
| No extra feature flags in WASM build | `testutils` feature excluded from WASM build step |
| `Cargo.lock` committed and unchanged | CI `build` job "Verify Cargo.lock" step + `cargo_lock_determinism` tests |
