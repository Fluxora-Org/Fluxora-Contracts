# Fluxora Maintainer Security Checklist

**Contract:** `FluxoraStream` + `FluxoraFactory` · **Platform:** Soroban (Stellar)
**Version:** 2 · **Last reviewed:** 2026-04-23

This checklist is for maintainers reviewing PRs, preparing releases, or auditing the
contract after any change. Work through every section before merging a change that
touches contract logic, storage layout, events, or admin powers.

---

## 1. CEI Pattern (Checks-Effects-Interactions)

Every entrypoint that moves tokens must follow this strict ordering:

```
1. Checks   — auth, state guards, parameter validation
2. Effects  — all state mutations + save_stream / save_*
3. Interactions — pull_token / push_token (external call last)
```

### 1.1 Per-entrypoint CEI audit

| Entrypoint                | Direction             | State saved before transfer?                                                | Notes                                                                |
| ------------------------- | --------------------- | --------------------------------------------------------------------------- | -------------------------------------------------------------------- |
| `create_stream`           | IN (pull)             | ✅ `persist_new_stream` after `pull_token` succeeds                         | If pull fails, no ID allocated, no state written                     |
| `create_streams`          | IN (bulk pull)        | ✅ Validate all → single pull → persist all                                 | Atomic batch; any failure reverts everything                         |
| `withdraw`                | OUT (push)            | ✅ `withdrawn_amount` + optional `Completed` → `save_stream` → `push_token` | CEI comment in source                                                |
| `withdraw_to`             | OUT (push)            | ✅ Same as `withdraw`; destination ≠ contract address checked first         |                                                                      |
| `batch_withdraw`          | OUT (push per stream) | ✅ Per-iteration: state saved → `push_token`                                | Each iteration is independently CEI-compliant                        |
| `cancel_stream`           | OUT (refund)          | ✅ `Cancelled` + `cancelled_at` → `save_stream` → `push_token`              | Shared via `cancel_stream_internal`                                  |
| `cancel_stream_as_admin`  | OUT (refund)          | ✅ Same — delegates to `cancel_stream_internal`                             | Identical externally visible semantics                               |
| `shorten_stream_end_time` | OUT (refund)          | ✅ `end_time` + `deposit_amount` → `save_stream` → `push_token`             | Refund skipped if `refund_amount == 0`                               |
| `top_up_stream`           | IN (pull)             | ✅ `deposit_amount` → `save_stream` → `pull_token`                          | Intentional reversal: state first prevents double-credit on re-entry |
| `trigger_auto_claim`      | OUT (push)            | ✅ `withdrawn_amount` + optional `Completed` → `save_stream` → `push_token` | Destination read from storage; caller cannot influence it            |

### 1.2 CEI review checklist

- [ ] No `pull_token` or `push_token` call appears before a `save_stream` call in the same entrypoint
- [ ] No state mutation occurs after any external token call
- [ ] `cancel_stream_internal` is the single code path for all cancellation logic (no duplicated cancel logic)
- [ ] `batch_withdraw` saves each stream individually before its corresponding `push_token`
- [ ] `top_up_stream` increments `deposit_amount` and calls `save_stream` **before** `pull_token`
- [ ] Any new entrypoint that moves tokens has a CEI comment in source and is added to the table above
- [ ] Every token-transfer entrypoint acquires `acquire_reentrancy_lock` before the external call and releases it with `release_reentrancy_lock` after
- [ ] `acquire_reentrancy_lock` is called **before** any state mutation that precedes a token transfer
- [ ] `release_reentrancy_lock` is called after every `push_token` / `pull_token` call completes (or deferred via a guard pattern)
- [ ] `DataKey::ReentrancyLock` is checked before setting; if already `true` the call reverts with `ContractError::InvalidState`
- [ ] A new entrypoint that moves tokens uses both CEI ordering **and** the explicit reentrancy lock — CEI alone is insufficient defense-in-depth

> **Note from audit cross-reference:** `CEI_ANALYSIS.md` (Issue #262) documents `withdraw`, `withdraw_to`, `batch_withdraw`, `cancel_stream`, and `cancel_stream_as_admin` as wrapped in the reentrancy lock, but the current code only wraps `sweep_excess` and `trigger_auto_claim`. Audit Invariant #13 (Reentrancy Guard) in `docs/audit.md` remains underspecified. See §14 for the open finding.

---

## 2. Authorization Boundaries

### 2.1 Role matrix

| Operation                                          | Sender | Recipient |          Admin          |       Anyone        |
| -------------------------------------------------- | :----: | :-------: | :---------------------: | :-----------------: |
| `create_stream` / `create_streams`                 |   ✅   |           |                         |                     |
| `pause_stream` / `resume_stream`                   |   ✅   |           |                         |                     |
| `cancel_stream`                                    |   ✅   |           |                         |                     |
| `withdraw` / `withdraw_to`                         |        |    ✅     |                         |                     |
| `batch_withdraw` / `batch_withdraw_to`             |        |    ✅     |                         |                     |
| `update_rate_per_second`                           |   ✅   |           |                         |                     |
| `decrease_rate_per_second`                         |   ✅   |           |                         |                     |
| `shorten_stream_end_time`                          |   ✅   |           |                         |                     |
| `extend_stream_end_time`                           |   ✅   |           |                         |                     |
| `top_up_stream`                                    |        |           |                         |  ✅ (any `funder`)  |
| `set_auto_claim` / `revoke_auto_claim`             |        |    ✅     |                         |                     |
| `trigger_auto_claim`                               |        |           |                         | ✅ (permissionless) |
| `pause_stream_as_admin` / `resume_stream_as_admin` |        |           |           ✅            |                     |
| `cancel_stream_as_admin`                           |        |           |           ✅            |                     |
| `set_global_emergency_paused`                      |        |           |           ✅            |                     |
| `set_contract_paused`                              |        |           |           ✅            |                     |
| `pause_protocol` / `resume_protocol`               |        |           |           ✅            |                     |
| `set_admin`                                        |        |           | ✅ (current admin only) |                     |
| `close_completed_stream`                           |        |           |                         | ✅ (permissionless) |
| `get_last_pause_record`                            |        |           |                         | ✅ (read-only)      |
| All `get_*` / `calculate_*` / `version`            |        |           |                         |   ✅ (read-only)    |

### 2.2 Auth boundary checklist

- [ ] Every state-mutating entrypoint calls `require_auth()` on the correct role before any state read or write
- [ ] `require_stream_sender` is used (not inline comparison) for all sender-gated operations
- [ ] `stream.recipient.require_auth()` is called at the top of `withdraw`, `withdraw_to`, and `set_auto_claim`
- [ ] `batch_withdraw` calls `recipient.require_auth()` once at entry, then verifies per-stream ownership
- [ ] Admin entrypoints call `get_admin(&env)?.require_auth()` — not a hardcoded address
- [ ] `top_up_stream` only requires `funder.require_auth()` — no sender restriction (intentional; document if changed)
- [ ] `trigger_auto_claim` has **no** `require_auth()` call — permissionless by design
- [ ] `close_completed_stream` has **no** `require_auth()` call — permissionless by design
- [ ] No entrypoint accepts an `admin` parameter that bypasses `get_admin()` storage lookup
- [ ] `set_admin` requires the **current** admin's auth, not the new admin's

### 2.3 Cross-role boundary violations to watch for

- Recipient must never be able to cancel or pause a stream
- Sender must never be able to withdraw (recipient-only)
- Admin cancel/pause/resume must route through `cancel_stream_internal` / shared helpers — no separate logic
- `top_up_stream` must not restrict `funder` to sender (treasury workflows depend on open funding)

---

## 3. Terminal State Gating

Terminal states (`Completed`, `Cancelled`) are irreversible. Any entrypoint that
mutates stream state must reject terminal-state streams before doing any work.

### 3.1 Terminal state transition table

```
Active    → Paused      (pause_stream / pause_stream_as_admin)
Active    → Cancelled   (cancel_stream / cancel_stream_as_admin)
Active    → Completed   (withdraw drains deposit_amount == withdrawn_amount)
Paused    → Active      (resume_stream / resume_stream_as_admin)
Paused    → Cancelled   (cancel_stream / cancel_stream_as_admin)
Completed → (terminal)  only close_completed_stream may act on it
Cancelled → (terminal)  withdraw still works (drains accrued_at_cancel); no other mutations
```

### 3.2 Terminal gating checklist

- [ ] `pause_stream` rejects `Completed` and `Cancelled` with `InvalidState`
- [ ] `resume_stream` rejects anything that is not `Paused`
- [ ] `cancel_stream` / `cancel_stream_as_admin` reject `Completed` and `Cancelled`
- [ ] `update_rate_per_second` / `decrease_rate_per_second` reject terminal states
- [ ] `shorten_stream_end_time` / `extend_stream_end_time` reject terminal states
- [ ] `top_up_stream` rejects `Completed` and `Cancelled`
- [ ] `withdraw` / `withdraw_to` reject `Paused` with `InvalidState`; allow `Cancelled` (drain accrued)
- [ ] `batch_withdraw` aborts the entire batch if any stream is `Paused`; skips `Completed` silently
- [ ] `close_completed_stream` rejects anything that is not `Completed`
- [ ] `trigger_auto_claim` rejects `Completed` and `Cancelled` with `InvalidState`
- [ ] No entrypoint transitions a `Cancelled` stream to `Completed` (cancelled streams stay `Cancelled` even when fully drained)
- [ ] `is_terminal_state` helper is used consistently — not reimplemented inline

### 3.3 Cancellation fee audit items

The optional cancellation fee (`cancellation_fee_bps > 0`) applies only to the unstreamed refund, never to the recipient's accrued amount. These items come from the auditor checklist in `docs/security.md`.

- [ ] `fee = (refund × fee_bps) / 10000` truncates down (no rounding up)
- [ ] Accrued calculation is independent of `cancellation_fee_bps` — fee never reduces `calculate_accrued` output
- [ ] Recipient's `withdraw` receives full accrued amount, not reduced by the fee
- [ ] Fee is never applied to or deducted from the accrued (recipient) portion
- [ ] State is persisted before any token transfer involving fee deduction (CEI compliance)

---

## 4. Version Bump Triggers

`CONTRACT_VERSION` (currently `2`) must be incremented before deploying any change
that breaks backward compatibility for integrators, indexers, or wallets.

### 4.1 Breaking changes that REQUIRE a version bump

| Category                                          | Examples                                                                |
| ------------------------------------------------- | ----------------------------------------------------------------------- |
| Removed or renamed entrypoint                     | Deleting `batch_withdraw`, renaming `withdraw_to`                       |
| Changed parameter order or type                   | Swapping `sender`/`recipient` args, changing `i128` → `u128`            |
| Changed `ContractError` discriminant              | Reordering enum variants, inserting a new code in the middle            |
| Changed event payload shape                       | Adding/removing/renaming a field in `StreamCreated`, `Withdrawal`, etc. |
| Changed `DataKey` discriminant                    | Reordering `DataKey` variants (see §5 for full rules)                   |
| New storage key that makes old entries unreadable | Any `DataKey` variant whose addition shifts existing discriminants      |
| Changed accrual formula observable output         | Different result for same inputs at same ledger time                    |

### 4.2 Changes that do NOT require a version bump

| Category                                                         | Notes                                                               |
| ---------------------------------------------------------------- | ------------------------------------------------------------------- |
| New additive entrypoint                                          | Old clients can ignore it; still recommended to bump conservatively |
| Internal refactor, identical external behaviour                  | Gas optimisations, helper extraction                                |
| Tightened validation (rejecting a previously-accepted edge case) | Document the change; no bump required                               |
| TTL constant changes                                             | Not observable by integrators                                       |
| Documentation-only changes                                       |                                                                     |

### 4.3 Version bump checklist

- [ ] `CONTRACT_VERSION` in `lib.rs` is incremented for any breaking change listed in §4.1
- [ ] The `DataKey` discriminant table comment in `lib.rs` is updated to reflect any new variants
- [ ] `wasm/checksums.sha256` is regenerated via `bash script/update-wasm-checksums.sh`
- [ ] `CHANGELOG.md` has an entry describing the breaking change and migration path
- [ ] Migration notes in the `CONTRACT_VERSION` doc comment are updated
- [ ] Deployment scripts reference the new version and verify `version()` on-chain after deploy

---

## 5. Event Compatibility

Events are the primary integration surface for indexers, wallets, and treasury tooling.
Any change to event topics, payload types, or emission order is a breaking change.

### 5.1 Event catalogue

| Topic       | Payload type                   | Emitting entrypoints                        | Notes                                              |
| ----------- | ------------------------------ | ------------------------------------------- | -------------------------------------------------- |
| `created`   | `StreamCreated`                | `create_stream`, `create_streams`           | One event per stream in batch                      |
| `withdrew`  | `Withdrawal`                   | `withdraw`, `batch_withdraw`                | Not emitted if `withdrawable == 0`                 |
| `wdraw_to`  | `WithdrawalTo`                 | `withdraw_to`                               | Both `recipient` and `destination` recorded        |
| `paused`    | `StreamEvent::Paused`          | `pause_stream`, `pause_stream_as_admin`     |                                                    |
| `resumed`   | `StreamEvent::Resumed`         | `resume_stream`, `resume_stream_as_admin`   |                                                    |
| `cancelled` | `StreamEvent::StreamCancelled` | `cancel_stream`, `cancel_stream_as_admin`   | Emitted after refund transfer                      |
| `completed` | `StreamEvent::StreamCompleted` | `withdraw`, `withdraw_to`, `batch_withdraw` | Always after `withdrew`/`wdraw_to` in same tx      |
| `closed`    | `StreamEvent::StreamClosed`    | `close_completed_stream`                    | Emitted before storage deletion                    |
| `rate_upd`  | `RateUpdated`                  | `update_rate_per_second`                    |                                                    |
| `rate_dec`  | `RateDecreased`                | `decrease_rate_per_second`                  | Includes `checkpointed_amount` and `refund_amount` |
| `end_shrt`  | `StreamEndShortened`           | `shorten_stream_end_time`                   |                                                    |
| `end_ext`   | `StreamEndExtended`            | `extend_stream_end_time`                    |                                                    |
| `top_up`    | `StreamToppedUp`               | `top_up_stream`                             | Includes `new_end_time` for indexer correlation    |
| `AdminUpd`  | `(old_admin, new_admin)`       | `set_admin`                                 |                                                    |
| `gl_pause`  | `GlobalEmergencyPauseChanged`  | `set_global_emergency_paused`               |                                                    |

### 5.2 Event ordering guarantees (within a single transaction)

```
1. withdrew / wdraw_to   (transfer confirmation)
2. completed             (if stream reaches terminal state via withdrawal)
3. cancelled             (if cancellation triggered refund)
4. closed                (always last; storage deleted after this)
```

`created` is always the only event in a stream-creation transaction.
For `create_streams`, one `created` event per stream entry, in input order.

### 5.3 Event compatibility checklist

- [ ] No existing event topic string (`"created"`, `"withdrew"`, etc.) is renamed or removed
- [ ] No field is added, removed, or reordered in any `#[contracttype]` event payload struct
- [ ] New events use a new `symbol_short!` topic that does not collide with existing topics
- [ ] `completed` is always emitted **after** `withdrew`/`wdraw_to` in the same transaction
- [ ] `closed` is always the last event in any transaction that calls `close_completed_stream`
- [ ] `cancel_stream` and `cancel_stream_as_admin` emit identical `cancelled` event shapes
- [ ] `pause_stream` and `pause_stream_as_admin` emit identical `paused` event shapes
- [ ] `resume_stream` and `resume_stream_as_admin` emit identical `resumed` event shapes
- [ ] Events are not emitted for no-op paths (e.g. `withdraw` with `withdrawable == 0`)
- [ ] `batch_withdraw` emits one `withdrew` per stream (not one aggregate event)
- [ ] Any new entrypoint that changes state emits exactly one primary event

---

## 6. Storage Key (`DataKey`) Safety

`DataKey` is serialised by Soroban using **discriminant index** (0-based, declaration order).
Reordering or inserting variants shifts all subsequent discriminants and silently corrupts
all existing persistent storage entries.

### 6.1 Current discriminant assignments (must never change)

| Discriminant | Variant                     | Storage type | Status |
| ------------ | --------------------------- | ------------ | ------ |
| 0            | `Config`                    | Instance     | Active |
| 1            | `NextStreamId`              | Instance     | Active |
| 2            | `Stream(u64)`               | Persistent   | Active |
| 3            | `RecipientStreams(Address)` | Persistent   | Active |
| 4            | `GlobalEmergencyPaused`     | Instance     | Active |
| 5            | `CreationPaused`            | Instance     | Active |
| 6            | `AutoClaimDestination(u64)` | Persistent   | Active |
| 7            | `GlobalPauseReason`         | Instance     | Active |
| 8            | `GlobalPauseTimestamp`      | Instance     | Active |
| 9            | `GlobalPauseAdmin`          | Instance     | Active |

### 6.2 DataKey checklist

- [ ] No existing `DataKey` variant is reordered or removed
- [ ] New variants are appended at the **end** of the enum only
- [ ] The discriminant table comment in `lib.rs` is updated for any new variant
- [ ] `CONTRACT_VERSION` is incremented when a new variant is added
- [ ] Deprecated variants are marked with a doc comment — never deleted from the enum
- [ ] Any new persistent key has TTL bump logic on both read and write paths

---

## 7. Arithmetic Safety

- [ ] All `rate_per_second × duration` multiplications use `checked_mul`
- [ ] All deposit accumulations in `create_streams` use `checked_add`
- [ ] `top_up_stream` uses `checked_add` on `deposit_amount`
- [ ] `cancel_stream_internal` refund uses `checked_sub` (underflow → `InvalidState`)
- [ ] `accrual::calculate_accrued_amount` result is capped at `deposit_amount` (never exceeds deposit)
- [ ] No `i128` arithmetic uses unchecked operators (`+`, `*`) on user-supplied values
- [ ] Fuzz harness (`accrual_fuzz`) passes with `PROPTEST_CASES=10000` before release

---

## 8. Global Pause State

A unified pause mechanism exists via the `PauseState` enum.

| State | Blocks | Does NOT block |
|---|---|---|
| `Active` | Nothing | Everything |
| `CreationPaused` | `create_stream`, `create_streams` | Everything else |
| `GlobalEmergencyPaused` | All user mutations (withdraw, cancel, pause, resume, rate updates, top-up, auto-claim) | Admin overrides (`*_as_admin`), views, `close_completed_stream`, `set_admin` |

- [ ] `require_not_globally_paused` is called at the top of every user-facing mutation entrypoint
- [ ] `require_creation_allowed` (creation gate) is called in `create_stream` and `create_streams`
- [ ] Admin entrypoints (`*_as_admin`, `set_global_emergency_paused`, `set_admin`) do **not** call `require_not_globally_paused`
- [ ] `close_completed_stream` does **not** call `require_not_globally_paused` (permissionless cleanup must remain available)
- [ ] `set_admin` is not blocked by any pause state (admin rotation must work under full freeze)
- [ ] `get_pause_info()` returns accurate state including the current `PauseState`

---

## 9. Factory Contract Checks

The factory (`FluxoraFactory`) is a thin policy layer over the stream contract.

- [ ] `set_allowlist` is admin-only; no public path to add arbitrary recipients
- [ ] `set_cap` / `set_min_duration` / `set_stream_contract` / `set_admin` are all admin-only
- [ ] `create_stream` enforces allowlist check **before** calling the stream contract
- [ ] `create_stream` enforces `deposit_amount <= max_deposit` cap
- [ ] `create_stream` enforces `end_time - start_time >= min_duration`
- [ ] Factory does not hold or custody tokens — it delegates directly to the stream contract
- [ ] `set_stream_contract` can point to an arbitrary address; verify the new address is a valid stream contract before calling in production

---

## 10. Pre-release Final Checks

Run these before tagging a release or deploying to testnet/mainnet.

### Build and test

```bash
# Full test suite
cargo test --workspace

# Fuzz accrual with high case count
PROPTEST_CASES=10000 cargo test -p fluxora_stream accrual_fuzz

# WASM build and checksum update
cargo build --release --target wasm32-unknown-unknown -p fluxora_stream
bash script/update-wasm-checksums.sh
```

### Checklist

- [ ] All tests pass (`cargo test --workspace`)
- [ ] No new compiler warnings introduced
- [ ] `wasm/checksums.sha256` updated and committed
- [ ] `CONTRACT_VERSION` incremented if any breaking change was made (see §4)
- [ ] `CHANGELOG.md` entry written with migration notes for integrators
- [ ] `DataKey` discriminant table in `lib.rs` is current
- [ ] All new entrypoints appear in the auth matrix (§2.1) and event catalogue (§5.1)
- [ ] All new entrypoints have corresponding integration tests in `contracts/stream/tests/integration_suite.rs`
- [ ] `docs/security.md` and `contracts/stream/SECURITY.md` updated if admin powers or trust model changed
- [ ] Deployment checklist in `docs/mainnet.md` reviewed for any new deployment steps

---

## 11. Quick Reference: What Requires What

| You changed…                                    | Required actions                                                              |
| ----------------------------------------------- | ----------------------------------------------------------------------------- |
| Entrypoint parameter order or type              | Bump `CONTRACT_VERSION`, update `CHANGELOG.md`, update auth matrix            |
| Event payload struct field                      | Bump `CONTRACT_VERSION`, update event catalogue (§5.1), update `CHANGELOG.md` |
| `ContractError` variant order                   | Bump `CONTRACT_VERSION`, update error reference in `CEI_ANALYSIS.md`          |
| `DataKey` enum (new variant)                    | Append only, bump `CONTRACT_VERSION`, update discriminant table               |
| Admin powers (new or removed)                   | Update `SECURITY.md`, update auth matrix (§2.1), update `docs/security.md`    |
| Accrual formula                                 | Update fuzz harness, update `docs/streaming.md`, bump `CONTRACT_VERSION`      |
| TTL constants                                   | No version bump; document in `CHANGELOG.md`                                   |
| Factory policy (cap, duration, allowlist logic) | Update factory tests, update §9 of this document                              |
| Token address or trust model                    | Update `docs/token-assumptions.md` and `docs/security.md`                     |

---

---

## 12. Snapshot Security Diff

Before merging any PR that modifies snapshot files under
`contracts/stream/test_snapshots/`, run `script/check_snapshot_diff.py` to
detect security-relevant field changes. The script classifies changes to admin
addresses, token identity, rate caps, pause state, recipient rotation, nonces,
and storage key layout against the `SECURITY_FIELDS` registry.

```bash
# Extract the base version from main
git show origin/main:contracts/stream/test_snapshots/test/test_NAME.1.json \
  > /tmp/base.json

# Run the classifier
python script/check_snapshot_diff.py \
  --base /tmp/base.json \
  --head contracts/stream/test_snapshots/test/test_NAME.1.json
```

### Exit-code contract

| Code | Meaning |
|---|---|
| `0` | No security-relevant changes. Standard review applies. |
| `1` | Security-relevant changes detected. Mandatory extra review required. |
| `2` | Usage error — bad path, invalid JSON, or wrong JSON type. |

### Checklist

- [ ] `check_snapshot_diff.py` has been run for every changed snapshot file in this PR
- [ ] The exit code is recorded in the PR description or review comment
- [ ] If exit code was `1`, all applicable mandatory extra review items from
  `docs/snapshot-security-diff.md` have been completed and documented

> **Note:** This tool is **not yet wired into CI**. The companion CI-wiring
> issue must land before it is enforced automatically. Until then, run it
> manually for every PR that touches snapshot files.

For the full field classification reference, worked examples, and the complete
reviewer workflow, see **[`docs/snapshot-security-diff.md`](snapshot-security-diff.md)**.

---

## 13. Historical Audit Invariants

The invariants below are extracted from `docs/audit.md` and were validated during the
initial contract audit. Each invariant that resulted in (or was confirmed by) a code
change is listed here so future reviewers do not miss a category of issue previously
found in this codebase.

### 13.1 Invariants with checklist coverage (existing items suffice)

| # | Invariant | Existing coverage |
|---|---|---|
| 1 | Accrued never exceeds deposit (`calculate_accrued` clamped) | §7 Arithmetic Safety |
| 3 | Only the recipient can withdraw | §2 Auth Boundary |
| 10 | Pause/resume/cancel authorization per role | §2 Auth Boundary |
| 11 | Status transitions follow the state machine in §3.1 | §3 Terminal State Gating |

### 13.2 Invariants requiring explicit checklist items

- [ ] **#2 — Withdrawn amount never exceeds deposit**: `withdrawn_amount` is only increased by `withdraw`/`withdraw_to`/`batch_withdraw` by the withdrawable amount (accrued − withdrawn), and `Completed` is set exactly when `withdrawn_amount == deposit_amount`; no further withdrawals allowed after terminal state
- [ ] **#4 — Stream IDs are unique**: IDs are assigned from a monotonically increasing `NextStreamId` counter; no reuse, no gap-fill, no decrement
- [ ] **#5 — Sender ≠ recipient**: `create_stream` and all creation entrypoints enforce `sender != recipient`; self-streaming is rejected
- [ ] **#6 — Deposit covers total streamable amount**: `create_stream` enforces `deposit_amount >= rate_per_second × (end_time − start_time)` with `checked_mul`
- [ ] **#7 — Deposit sufficiency preserved on extension**: `extend_stream_end_time` re-validates `deposit_amount >= rate_per_second × (new_end_time − start_time)` before mutation; caller must `top_up_stream` first if deposit is insufficient
- [ ] **#8 — Time bounds**: `start_time < end_time` and `cliff_time ∈ [start_time, end_time]` are enforced in every creation entrypoint
- [ ] **#9 — Init once (authenticated bootstrap)**: `init` panics if `Config` already exists; requires `admin.require_auth()`; token is immutable after init
- [ ] **#12 — Cancellation timestamp and refund semantics**: `cancelled_at` is set to current ledger time; accrual is frozen at `cancelled_at`; refund = `deposit_amount − accrued_at(cancelled_at)`; `cancel_stream` and `cancel_stream_as_admin` produce identical state/event semantics
- [ ] **#12 (cancel parity)** — `cancel_stream` and `cancel_stream_as_admin` route through the same `cancel_stream_internal` helper guaranteeing identical external behaviour
- [ ] **#14 — Contract balance consistency**: Deposit is pulled only in `create_stream`/`create_streams`/`top_up_stream`; refunds and withdrawals are derived from deposits; no minting, no arbitrary transfers

### 13.3 Resolved audit findings (from `docs/security.md`)

- [ ] **top_up_stream CEI fix**: `top_up_stream` was previously pulling tokens before persisting state (violating CEI). The fix reversed the order — state is now saved before `pull_token`. Verify any new `top_up_stream`-like entrypoint does not repeat this bug.

---

## 14. Resolved Findings — Invariant #13 (Reentrancy Guard)

The Invariant #13 finding previously listed here as **open / unaddressed** has been
resolved. This section documents the resolution for audit trail completeness.

### ✅ Invariant #13 — Reentrancy Guard (resolved 2026-07-27)

**Location:** `docs/audit.md` Invariant #13

**Previous status:** The invariant header existed but no requirements, checks, or
enforcement criteria were specified, and `CEI_ANALYSIS.md` (Issue #262) contained
inaccurate claims about which entrypoints held the explicit reentrancy lock.

**Resolution — CEI-only as accepted design:**

The accepted posture for reentrancy protection is documented in full in
`docs/audit.md` Invariant #13. Summary:

1. **Primary defence is CEI ordering.** All standard token-transfer entrypoints
   (`withdraw`, `withdraw_to`, `batch_withdraw`, `cancel_stream`,
   `cancel_stream_as_admin`, `keeper_cancel`, `top_up_stream`, etc.) persist all
   state changes to storage **before** any external token call. Because Soroban
   re-enters the contract only on an explicit cross-contract call, a correctly
   ordered CEI sequence is sufficient to prevent double-spend or state-corruption
   reentrancy on these paths.

2. **Explicit lock on two permissionless / admin-callable paths only.** `sweep_excess`
   and `trigger_auto_claim` acquire `DataKey::ReentrancyLock` in addition to CEI
   ordering. These are the only two entrypoints that warrant the lock (concurrent
   invocations are plausible and the lock prevents a race). Extending the lock to all
   entrypoints would introduce deadlock risk in legitimate same-transaction batch flows.

3. **`CEI_ANALYSIS.md` inaccuracy corrected.** The claim that `withdraw`,
   `withdraw_to`, `batch_withdraw`, `cancel_stream`, and `cancel_stream_as_admin` are
   wrapped in the explicit lock was inaccurate. Those entrypoints rely solely on CEI
   ordering, which is the intended and sufficient design. `CEI_ANALYSIS.md` is
   superseded by `docs/audit.md` Invariant #13 and this resolution note.

**Checklist items verifying the resolved design:**

- [x] All standard token-transfer entrypoints follow strict CEI order (state saved
      before any `push_token` / `pull_token` call)
- [x] `sweep_excess` and `trigger_auto_claim` acquire/release `DataKey::ReentrancyLock`
- [x] No other entrypoint acquires `DataKey::ReentrancyLock`
- [x] `docs/audit.md` Invariant #13 now contains the full specification
- [x] `CEI_ANALYSIS.md` claim corrected by this resolution note

**No code changes were required.** The implementation already matched the intended
design. Only documentation was out of sync.

---

## 15. Build and Gas Determinism Guarantees

This section covers the determinism invariants that make WASM builds, gas baselines, and
upgrade behaviour reproducible across machines, CI runs, and retries. These invariants
are a prerequisite for auditor verification and for the gas-baseline comparison in
`script/validate_gas.py` to produce stable, meaningful results.

### 15.1 Determinism invariant table

| Invariant | Mechanism | Verified by |
|-----------|-----------|-------------|
| Rust toolchain version is fixed | `rust-toolchain.toml` pins to `1.94.1` | `script/verify_rust_version.py` (every CI job) |
| `soroban-sdk` is exact-pinned, not range-pinned | `contracts/stream/Cargo.toml` uses `"21.7.7"` not `"^21.7.7"` | `cargo_lock_determinism` Rust test |
| All transitive dependencies are locked | `Cargo.lock` committed and unchanged | `cargo update --locked` CI gate (build job) |
| Build profile is `--release --target wasm32-unknown-unknown` | CI `build` job invocation | WASM artifact upload step |
| No test features bleed into WASM build | `testutils` feature excluded from WASM build step | CI `build` job configuration |
| WASM checksum matches reference after build | `wasm/checksums.sha256` committed | `bash script/verify-wasm-checksum.sh --no-build` |
| Gas baselines are stable across retries | Soroban metered host is deterministic | `script/validate_gas.py` (gas regression CI step) |

For the full Cargo.lock determinism contract, recovery procedure, and security
assumptions, see `docs/upgrade.md §8`.

### 15.2 Per-upgrade determinism checklist

Run these checks before tagging any release that changes the toolchain, SDK version, or
contract source:

- [ ] `rustc --version` matches the version in `rust-toolchain.toml` in this environment
- [ ] `cargo update --locked --workspace` exits 0 (no dependency drift)
- [ ] `bash script/verify-wasm-checksum.sh --no-build` passes with the committed artifact
- [ ] If toolchain or SDK version changed: gas baselines in `docs/gas.md` have been
      re-measured and the JSON block updated (`script/validate_gas.py` passes)
- [ ] If toolchain or SDK version changed: `bash script/update-wasm-checksums.sh` has been
      run and `wasm/checksums.sha256` committed
- [ ] `script/validate_gas.py` passes on the current commit without any `FAIL` lines
- [ ] The `PausedStreamCount` backfill caveat (docs/upgrade.md §3, docs/gas.md
      "Release Hardening") has been reviewed if upgrading from a pre-v5 instance

### 15.3 Gas baseline retry behaviour

Gas baselines are **deterministic across retries**. Re-running `script/validate_gas.py`
on an unchanged commit and unchanged toolchain always produces the same pass/fail result
because:

1. The Soroban metered host counts CPU instructions deterministically from WASM bytecode.
2. The test harness (`soroban_sdk::testutils`) runs in-process with a fixed ledger state.
3. No external network calls, system entropy, or wall-clock time influence the counts.

If `validate_gas.py` produces a different result on a second run, suspect a toolchain
mismatch or an uncommitted file change rather than flaky test infrastructure.

### 15.4 WASM checksum role in security

`wasm/checksums.sha256` is the authoritative reference for deployment verification.
It must be updated whenever the WASM binary changes (source change, toolchain bump,
or SDK bump). The verification workflow:

```bash
# Verify a build matches the committed reference (no rebuild):
bash script/verify-wasm-checksum.sh --no-build

# Rebuild and update the reference after a source change:
cargo build --release -p fluxora_stream --target wasm32-unknown-unknown
bash script/update-wasm-checksums.sh
git add wasm/checksums.sha256
git commit -m "chore: update wasm checksums"
```

A deployed contract whose on-chain bytecode hash does not match `wasm/checksums.sha256`
should be treated as unverified until the discrepancy is explained.

---

_Review this document after any contract upgrade, admin power change, or storage layout modification._
_File location: `docs/maintainer-security-checklist.md`_
