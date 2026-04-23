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

## 8. Global Pause Flags

Two independent pause mechanisms exist. Understand which operations each blocks.

| Flag                    | `DataKey`                        | Blocks                                                                                 | Does NOT block                                                               |
| ----------------------- | -------------------------------- | -------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------- |
| `CreationPaused`        | `DataKey::CreationPaused`        | `create_stream`, `create_streams`                                                      | Everything else                                                              |
| `GlobalEmergencyPaused` | `DataKey::GlobalEmergencyPaused` | All user mutations (withdraw, cancel, pause, resume, rate updates, top-up, auto-claim) | Admin overrides (`*_as_admin`), views, `close_completed_stream`, `set_admin` |

- [ ] `require_not_globally_paused` is called at the top of every user-facing mutation entrypoint
- [ ] `require_not_paused` (creation gate) is called in `create_stream` and `create_streams`
- [ ] Admin entrypoints (`*_as_admin`, `set_global_emergency_paused`, `set_admin`) do **not** call `require_not_globally_paused`
- [ ] `close_completed_stream` does **not** call `require_not_globally_paused` (permissionless cleanup must remain available)
- [ ] `set_admin` is not blocked by `GlobalEmergencyPaused` (admin rotation must work under full freeze)
- [ ] `get_pause_info()` returns accurate state for both flags

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

_Review this document after any contract upgrade, admin power change, or storage layout modification._
_File location: `docs/maintainer-security-checklist.md`_
