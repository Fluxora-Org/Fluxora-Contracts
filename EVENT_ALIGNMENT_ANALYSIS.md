# Event Catalog Alignment Analysis

| Analysis metadata      | Value                                                                                  |
| ---------------------- | -------------------------------------------------------------------------------------- |
| **Last performed**     | **2026-07-29**                                                                         |
| **Commit**             | `046ad40934e86f908c74c17968940d02baa4336a` (`046ad40`, `main`)                         |
| **Contract source**    | `contracts/stream/src/lib.rs` (9,038 lines), `contracts/stream/src/events.rs` (189 lines), `contracts/stream/src/storage.rs` |
| **Catalog source**     | `docs/events.md` (856 lines)                                                           |
| **Supersedes**         | The previous undated snapshot of this file, which cited `persist_new_stream` at line 474 (now line **1,649** — ~1,175 lines of drift) |

> **Staleness contract:** every line citation in this document was re-derived from the
> commit above on the date above using the commands in §Methodology. If
> `contracts/stream/src/lib.rs`, `contracts/stream/src/events.rs`, or `docs/events.md`
> has changed since `046ad40`, **do not trust the line numbers or verdicts below** —
> re-run the analysis (see §Regenerating this analysis) before citing this file.

## Issue Summary

Events catalog: align symbol names (and payload shapes) with `docs/events.md`.

The previous version of this document was a point-in-time snapshot whose line-number
citations silently drifted as the contract grew from ~2,200 to ~9,000 lines and as
event emission was refactored out of `lib.rs` into per-event helper functions in
`contracts/stream/src/events.rs`. Its central claim — that every call site was
"✅ Aligned" — could not be taken at face value because the citations no longer
pointed at the code they purported to describe. This version redoes the analysis
from scratch against the current source rather than patching stale numbers.

## Scope

Verify that **every event emission reachable from `contracts/stream/src/lib.rs`**
matches the documented event schemas in `docs/events.md` — topic tuple **and**
payload shape — so integrators can rely on consistent event topics and data
structures.

## Methodology

1. Enumerated every direct `env.events().publish(...)` call site in the current `lib.rs`:

   ```bash
   grep -n "\.publish(" contracts/stream/src/lib.rs
   ```

   > ⚠️ The issue's suggested `grep -n "events().publish"` is **insufficient**: two
   > call sites (`set_admin`, `register_stream_template`) split `env.events()` and
   > `.publish(` across two lines and are missed by that pattern. Matching
   > `.publish(` finds all 12 sites.

2. Enumerated every helper-mediated emission (the contract now emits most events
   through one-file-one-topic helpers):

   ```bash
   grep -n "events::emit_"  contracts/stream/src/lib.rs contracts/stream/src/storage.rs
   grep -n "\.publish("     contracts/stream/src/events.rs
   ```

3. Cross-checked each site's **topic tuple** and **payload struct/field types**
   against `docs/events.md`, recording *which* of the document's two tables each
   row was checked against (see below).

4. Verified "not emitted" claims by searching for the topic symbol across
   `contracts/stream/src/*.rs`.

### Which `docs/events.md` table is canonical?

`docs/events.md` contains **two** event tables that contradict each other in
several places (this self-contradiction is tracked separately in this batch):

| Table | Location in `docs/events.md` | Status in this analysis |
| --- | --- | --- |
| **Canonical** — the `## Event list` table (lines 15–57) plus detailed schema sections §1–§15 | Top of file; introduced as "the canonical source of truth for indexers and backend parsers … derived directly from the contract source" | **Primary reference.** Every row below was checked against this table first (column `C`). |
| **Stale legacy** — the `| Source location | Symbol emitted |` table appended after the "Commit message suggestion" line (lines 806–846) | Bottom of file; function names have drifted from the code (e.g. `resume_global_pause` → actual `global_resume`; `set_decommissioned` → actual `set_stream_decommissioned`) and it attributes `withdrew` to `batch_withdraw`, which the code no longer does | **Secondary cross-check only** (column `L`). Where `C` and `L` disagree, the code was treated as ground truth and the disagreement is reported in §Findings. |

Known contradictions **between the two tables / detail sections** (code is ground truth):

| Event | Canonical table | Legacy table / detail section | Actual code |
| --- | --- | --- | --- |
| `AdminUpdated` | `["AdminUpd"]` (line 36) | Legacy table: `"AdminUpd"` (line 821) — agrees; **but §9 (lines 259–272) says `["AdminUpdated"]`** | `symbol_short!("AdminUpd")` at `lib.rs:4455–4456` → canonical + legacy table correct, **§9 is stale** |
| `ContractPauseChanged` | `["ct_pause"]` (line 37) | **§15 schema (line 572) says `["paused_ctl"]`** | `symbol_short!("ct_pause")` at `events.rs:128–130` → canonical correct, **§15 is stale** |

Each inventory row below carries a **Checked against** column: `C` = canonical
`Event list` table (+ matching §-section), `L` = stale legacy source-location table,
`—` = no row exists in that table.

## Analysis Performed

### Table 1 — Direct `env.events().publish(...)` call sites in the current `lib.rs`

All 12 direct call sites found by `grep -n "\.publish(" contracts/stream/src/lib.rs`
(commit `046ad40`, 2026-07-29):

| # | Line(s)   | Function (def line)            | Topic tuple                              | Payload (type as emitted)                                                                                                   | Checked against | Verdict |
| - | --------- | ------------------------------ | ---------------------------------------- | --------------------------------------------------------------------------------------------------------------------------- | --------------- | ------- |
| 1 | 3737–3743 | `transfer_claim_ownership` (3714) | `("claim_own", stream_id)`             | `ClaimOwnershipTransferred { stream_id: u64, old_owner: Option<Address>, new_owner: Address }`                              | C ✅ / L ✅     | ✅ **Aligned** — topic and payload shape match the canonical `ClaimOwnershipTransferred` row. |
| 2 | 4455–4456 | `set_admin` (4440)             | `(symbol_short!("AdminUpd"),)`           | `(old_admin: Address, new_admin: Address)`                                                                                  | C ✅ / L ✅ (§9 ❌) | ✅ **Aligned vs. canonical + legacy tables** — the code-side fix recommended by the previous analysis (`Symbol::new("AdminUpdated")` → `symbol_short!`) is implemented. Residual drift is doc-internal: §9 of `docs/events.md` still prints `["AdminUpdated"]` (see Finding F4). |
| 3 | 5137–5146 | `delegate_recipient_share` (4988) | `("del_share", stream_id)`            | `RecipientShareDelegated { parent_stream_id, child_stream_id, delegator, delegatee, share_bps: u32, new_parent_rate: i128, child_rate: i128 }` | C ✅ / L ✅ | ✅ **Aligned** — matches canonical row field-for-field. |
| 4 | 5654–5660 | `renew_stream` (5580)          | `("renewed", stream_id, new_stream_id)`  | `StreamRenewed { old_stream_id: u64, new_stream_id: u64 }`                                                                  | C ✅ / L ✅     | ✅ **Aligned** — 3-element topic tuple matches canonical `["renewed", old, new]`. |
| 5 | 5841–5842 | `register_stream_template` (5810) | `("tmpl_def", template_id)`           | `StreamScheduleTemplate { template_id: u64, owner: Address, start_delay: u64, cliff_delay: u64, duration: u64 }`            | C **— (no row)** / L — | ❌ **Not Aligned** — emitted in code and mentioned only in the "Additional topics (validator)" line (`docs/events.md:59`), but the canonical `Event list` table has **no `tmpl_def` row** and there is no §-section with its payload schema (Finding F2). |
| 6 | 8061–8064 | `release_reservation` (8047)   | `("res_rel", holder)`                    | `(res.start_id: u64, res.count: u32, res.consumed: u32, reclaimed: u32)` — field types per `IdReservation` (`lib.rs:448–453`) | C ⚠️ / L —     | ❌ **Not Aligned (payload widths)** — topic matches, but canonical row (line 51) documents the tuple as `(u64, u64, u64, u64)` while the code emits `(u64, u32, u32, u32)` (Finding F1). |
| 7 | 8418–8432 | `create_stream_offer` (8329)   | `("offr_crt", offer_id)`                 | `StreamOfferCreated { offer_id, sender, recipient, deposit_amount: i128, rate_per_second: i128, start_time, cliff_time, end_time, expiry_time: Option<u64>, created_at }` | C ✅ / L ✅ | ✅ **Aligned** — matches canonical row field-for-field. |
| 8 | 8590–8597 | `accept_stream_offer` (8465)   | `("offr_acc", offer_id)`                 | `StreamOfferAccepted { offer_id: u64, effective_start_time: u64, recipient: Address }`                                      | C ✅ / L ✅     | ✅ **Aligned**. |
| 9 | 8639–8646 | `reject_stream_offer` (8616)   | `("offr_cxl", offer_id)`                 | `StreamOfferCancelled { offer_id: u64, by: Address (= recipient), refund_amount: i128 }`                                    | C ✅ / L ✅     | ✅ **Aligned**. |
| 10 | 8689–8696 | `cancel_stream_offer` (8666)  | `("offr_cxl", offer_id)`                 | `StreamOfferCancelled { offer_id: u64, by: Address (= sender), refund_amount: i128 }`                                       | C ✅ / L ✅     | ✅ **Aligned** (shares the `offr_cxl` row; `by` distinguishes actor). |
| 11 | 8817–8825 | `upgrade` (8794)               | `("upgraded",)`                          | `ContractUpgraded { new_wasm_hash: BytesN<32>, new_version: u32, upgraded_at: u64, upgraded_by: Address }`                  | C ✅ / L ✅     | ✅ **Aligned**. |
| 12 | 8828–8832 | `upgrade` (8794), legacy compat | `("upgrade",)`                          | `(new_wasm_hash: BytesN<32>, old_version: u32, new_version: u32, admin: Address)`                                           | C ✅ (noted in `ContractUpgraded` row, line 57) / L ✅ | ✅ **Aligned** — the canonical row explicitly documents this backward-compat second emission. |

### Table 2 — Helper-mediated emissions (`contracts/stream/src/events.rs`)

Emission was refactored: 28 helpers in `events.rs` each wrap one
`env.events().publish(...)`, plus `maybe_emit_health_changed` in `storage.rs:739`.
All helper topics use `symbol_short!`. Every helper's topic + payload shape was
re-verified against the **canonical** table (C); the legacy table (L) is shown for
cross-reference.

| Helper (events.rs)          | Publish line | Topic                 | Payload type                  | Call sites in `lib.rs` (commit `046ad40`) | C | L | Verdict |
| --------------------------- | ------------ | --------------------- | ----------------------------- | ----------------------------------------- | - | - | ------- |
| `emit_stream_created` (14)  | 15–16        | `("created", id)`     | `StreamCreated`               | 1722 (`persist_new_stream`), 1817 (`persist_new_stream_skip_index`), 2426 (`create_pooled_stream`), 8572 (`accept_stream_offer`) | ✅ | ✅ | ✅ Aligned (shape). Canonical "When emitted" omits the `accept_stream_offer` path — F6. |
| `emit_withdrawal` (20)      | 21–22        | `("withdrew", id)`    | `Withdrawal`                  | 3300 (`withdraw`), 3426 (`withdraw_from_pool`), 4128 (`delegated_withdraw`), 8239 (`bulk_cancel_streams`) | ✅ | ⚠️ | ✅ Aligned (shape). Legacy table attributes `withdrew` to `batch_withdraw` — **false**, see F3. Canonical omits `withdraw_from_pool` / `bulk_cancel_streams` emitters — F6. |
| `emit_withdrawal_to` (26)   | 27–28        | `("wdraw_to", id)`    | `WithdrawalTo`                | 3572 (`withdraw_to`), 3935 (`batch_withdraw_to`, incl. via `batch_withdraw` 3796→3808 delegation), 7603 (`trigger_auto_claim`) | ✅ | ✅ | ✅ Aligned (shape). Canonical "When emitted" omits `trigger_auto_claim` — F6. `batch_withdraw` emits this topic, not `withdrew` — F3. |
| `emit_stream_cancelled` (32)| 33–36        | `("cancelled", id)`   | `StreamEvent::StreamCancelled(id)` | 6349 (`cancel_stream_internal` ← `cancel_stream` 3106, `cancel_stream_as_admin` 6459), 8266 (`bulk_cancel_streams`) | ✅ | ✅ | ✅ Aligned (shape). Canonical omits `bulk_cancel_streams` — F6. |
| `emit_stream_completed` (40)| 41–44        | `("completed", id)`   | `StreamEvent::StreamCompleted(id)` | 3311 (`withdraw`), 3437 (`withdraw_from_pool`), 3584 (`withdraw_to`), 3947 (`batch_withdraw_to`), 4139 (`delegated_withdraw`), 7616 (`trigger_auto_claim`) | ✅ | ✅ | ✅ Aligned (shape). Canonical "When emitted" names only `withdraw`/`batch_withdraw` — F6. |
| `emit_stream_closed` (48)   | 49–52        | `("closed", id)`      | `StreamEvent::StreamClosed(id)` | 5724 (`close_completed_stream`), 5789 (`close_cancelled_stream`) | ✅ | ✅ | ✅ Aligned (shape). Canonical names only `close_completed_stream`; `close_cancelled_stream` also emits it — F6. |
| `emit_stream_paused` (56)   | 57–58        | `("paused", id)`      | `StreamPaused { stream_id, reason: String }` | 2980 (`pause_stream`), 6822 (`pause_stream_as_admin`) | ✅ | ✅ | ✅ Aligned — v3 `StreamPaused` shape (matches §4). |
| `emit_stream_resumed` (62)  | 63–66        | `("resumed", id)`     | `StreamEvent::Resumed(id)`    | 3046 (`resume_stream`), 6885 (`resume_stream_as_admin`), 6974 (`bulk_resume_streams_as_admin`) | ✅ | ✅ | ✅ Aligned (shape). Canonical omits `bulk_resume_streams_as_admin` — F6. |
| `emit_rate_updated` (70)    | 71–72        | `("rate_upd", id)`    | `RateUpdated`                 | 4801 (`update_rate_per_second`), 6406 (`update_rate`) | ✅ | ✅ | ✅ Aligned (shape). Canonical names only `update_rate_per_second`; `update_rate` also emits it — F6. |
| `emit_rate_decreased` (76)  | 77–78        | `("rate_dec", id)`    | `RateDecreased`               | 4962 (`decrease_rate_per_second`)         | ✅ | ✅ | ✅ Aligned. |
| `emit_rate_cap_enforced` (82) | 83–84      | `("rate_cap", id)`    | `RateCapEnforced`             | 4760 (`update_rate_per_second`, cap branch) | ✅ | ✅ | ✅ Aligned. |
| `emit_stream_end_shortened` (88) | 89–90   | `("end_shrt", id)`    | `StreamEndShortened`          | 5264 (`shorten_stream_end_time`)          | ✅ | ✅ | ✅ Aligned. |
| `emit_stream_end_extended` (94) | 95–96   | `("end_ext", id)`     | `StreamEndExtended`           | 5352 (`extend_stream_end_time`)           | ✅ | ✅ | ✅ Aligned. |
| `emit_stream_topped_up` (100) | 101–102  | `("top_up", id)`      | `StreamToppedUp`              | 5474 (`top_up_stream`)                    | ✅ | ✅ | ✅ Aligned. |
| `emit_stream_health_changed` (106) | 107–108 | `("health", id)` | `StreamHealthChanged`         | via `maybe_emit_health_changed` (`storage.rs:742`) from 6676 (`keeper_cancel`), 8268 (`bulk_cancel_streams`); also called from `decrease_rate_per_second`, `shorten_stream_end_time`, `top_up_stream`, `cancel_stream` paths | ✅ | ✅ | ✅ Aligned (shape). Canonical emitter list omits `keeper_cancel` and `bulk_cancel_streams` — F6. |
| `emit_recipient_updated` (112) | 113–114 | `("recp_upd", id)`  | `RecipientUpdated`            | 3677 (`accept_recipient_update`)          | ✅ | ✅ | ✅ Aligned (shape). Emission happens in `accept_recipient_update` (two-phase rotation), not directly in `update_recipient` (3599) — F6. |
| `emit_global_emergency_pause_changed` (118) | 119 | `("gl_pause",)` | `GlobalEmergencyPauseChanged` | 7000 (`set_global_emergency_paused`)    | ✅ | ✅ | ✅ Aligned. |
| `emit_global_resumed` (123) | 124          | `("gl_resume",)`      | `GlobalResumed`               | 7048 (`global_resume`)                    | ✅ | ⚠️ | ✅ Aligned vs. canonical. Legacy table cites the function as `resume_global_pause` — actual name is `global_resume` — F7. |
| `emit_contract_pause_changed` (128) | 129  | `("ct_pause",)`     | `ContractPauseChanged`        | 7078 (`set_contract_paused`)              | ✅ | ✅ (§15 ❌) | ✅ Aligned vs. canonical + legacy. §15 schema prints `["paused_ctl"]` — stale, see F4. |
| `emit_protocol_paused` (133) | 134–135     | `("pr_pause", admin)` | `ProtocolPaused`              | 7145 (`pause_protocol`)                   | ✅ | ✅ | ✅ Aligned. |
| `emit_protocol_resumed` (139) | 140–141    | `("pr_resume", admin)` | `ProtocolResumed`            | 7203 (`resume_protocol`)                  | ✅ | ✅ | ✅ Aligned. |
| `emit_auto_claim_set` (145) | 146–147      | `("ac_set", id)`      | `AutoClaimSet`                | 7412 (`set_auto_claim`)                   | ✅ | ✅ | ✅ Aligned. |
| `emit_auto_claim_revoked` (151) | 152–153  | `("ac_revoke", id)`   | `AutoClaimRevoked`            | 7458 (`revoke_auto_claim`)                | ✅ | ✅ | ✅ Aligned. |
| `emit_auto_claim_triggered` (157) | 158–159 | `("ac_trig", id)`  | `AutoClaimTriggered`          | 7592 (`trigger_auto_claim`)               | ✅ | ✅ | ✅ Aligned. |
| `emit_excess_swept` (163)   | 164–165      | `("ex_swept", recipient)` | `ExcessSwept`             | 7330 (`sweep_excess`)                     | ✅ | ✅ | ✅ Aligned. |
| `emit_stream_cloned` (169)  | 170–171      | `("cloned", id)`      | `StreamCloned`                | 7922 (`clone_stream`)                     | ✅ | ✅ | ✅ Aligned. |
| `emit_keeper_cancelled` (175) | 176–177    | `("kp_cncl", id)`     | `KeeperCancelled`             | 6664 (`keeper_cancel`)                    | ✅ | ✅ | ✅ Aligned (matches §13, incl. fee-identity comment). |
| `emit_stream_decommissioned` (181) | 182–188 | `("decomm", id)` | `StreamDecommissioned`        | 5531 (`set_stream_decommissioned`)        | ✅ | ⚠️ | ✅ Aligned vs. canonical. Legacy table cites function as `set_decommissioned` — actual name is `set_stream_decommissioned` — F7. |

### Table 3 — Documented-but-unemitted and emitted-but-undocumented events

| Topic | Status | Evidence | Verdict |
| ----- | ------ | -------- | ------- |
| `sndr_xfr` (`SenderTransferred`) | Canonical table row (line 40) + §12 claim emission by `transfer_sender`; struct defined at `lib.rs:858` and `types.rs:261` | `grep -rn "sndr_xfr" contracts/` → **zero matches**; no `transfer_sender` entrypoint exists anywhere in `contracts/` | ❌ **Not Aligned** — phantom documented event (F5). |
| `migrated` (`MigrationCheckpoint`) | Canonical row (line 50) explicitly states "No function currently emits this event. Reserved for future migration checkpoints." | `grep -rn "migrated\|MigrationCheckpoint" contracts/stream/src/*.rs` → no emission, no struct | ✅ **Aligned** — the doc's "not emitted" claim is accurate. |
| `tmpl_def` | Emitted at `lib.rs:5841–5842` | No canonical-table row; only the validator line (line 59) mentions it | ❌ **Not Aligned** — emitted but undocumented schema (F2). |

## Findings (freshly re-verified verdicts)

### F1 — `res_rel` payload width mismatch (code vs. canonical table) — ❌ Not Aligned

- **Code** (`lib.rs:8061–8064`): publishes `(res.start_id, res.count, res.consumed, reclaimed)`.
  `IdReservation` (`lib.rs:448–453`) defines `count: u32`, `consumed: u32`; `reclaimed` is a local `u32`.
  Effective on-chain shape: `(u64, u32, u32, u32)`.
- **Canonical table** (`docs/events.md:51`): documents `(start_id: u64, count: u64, consumed: u64, reclaimed: u64)`.
- **Impact**: indexers decoding `count`/`consumed`/`reclaimed` as `u64` per the doc will
  mis-parse the Soroban `Val` width. Recommend either (a) correcting the doc to `u32`, or
  (b) widening the emitted tuple to `u64` (breaking change for existing parsers — version-gate it).

### F2 — `tmpl_def` emitted but absent from the canonical table — ❌ Not Aligned

`register_stream_template` (`lib.rs:5841–5842`) emits
`("tmpl_def", template_id)` with `StreamScheduleTemplate { template_id: u64, owner: Address,
start_delay: u64, cliff_delay: u64, duration: u64 }`. The canonical `Event list` table has no
row for it (only the "Additional topics (validator)" line at `docs/events.md:59` lists the
symbol). Add a canonical row + schema section.

### F3 — `batch_withdraw` emits `wdraw_to`, not `withdrew` — ❌ Not Aligned (attribution)

`batch_withdraw` (`lib.rs:3796–3809`) builds `WithdrawToParam`s with `destination = recipient`
and delegates to `batch_withdraw_to` (`lib.rs:3811`), which emits `wdraw_to`/`WithdrawalTo`
(`lib.rs:3935`) — never `withdrew`. Both the canonical §2 ("Emitted by `withdraw` and each
stream in `batch_withdraw`") and the legacy table ("`withdraw`, `batch_withdraw`" → `withdrew`)
attribute the wrong topic to `batch_withdraw`. The topic/payload shapes themselves are aligned;
the function→topic mapping in the docs is what is wrong. Recommend: update docs to state that
`batch_withdraw` emits `wdraw_to` with `destination == recipient`.

### F4 — Doc-internal contradictions where code matches the canonical table — ⚠️ Docs stale

- **`AdminUpdated`**: code emits `symbol_short!("AdminUpd")` (`lib.rs:4455–4456`); canonical
  row (line 36) and legacy table (line 821) agree; **§9 (lines 259–272, incl. its JSON example)
  still says `["AdminUpdated"]`** — the exact inconsistency the previous analysis flagged, now
  fixed in code but not in §9.
- **`ContractPauseChanged`**: code emits `symbol_short!("ct_pause")` (`events.rs:128–130`);
  canonical row (line 37) agrees; **§15 schema (line 572) says `["paused_ctl"]`**.
- The previous analysis' "Priority 1: Code Alignment" (switch to `symbol_short!`) is therefore
  **done**; its "Priority 2: Documentation Correction" is **partially done** (canonical table
  updated; §9/§15 detail sections not).

### F5 — `sndr_xfr` / `SenderTransferred`: documented, never emitted — ❌ Not Aligned

Canonical row (line 40) + §12 describe emission by a `transfer_sender` entrypoint; the struct
is even defined (`lib.rs:858`, `types.rs:261`). But no `transfer_sender` function and no
`sndr_xfr` publish exist anywhere under `contracts/`. Either implement the entrypoint or mark
the row "reserved — not currently emitted" (as `migrated` is).

### F6 — Canonical "When emitted" / emitter lists are incomplete — ⚠️ Minor

Verified additional emitters missing from the canonical table's attribution text (shapes all aligned):

| Event | Additional actual emitters (lib.rs line) |
| --- | --- |
| `created` | `accept_stream_offer` (8572) |
| `withdrew` | `withdraw_from_pool` (3426), `delegated_withdraw` (4128, net-amount per the v9 indexer note), `bulk_cancel_streams` (8239) |
| `wdraw_to` | `trigger_auto_claim` (7603); `batch_withdraw` via delegation (3796→3935) |
| `completed` | `withdraw_to` (3584), `batch_withdraw_to` (3947), `withdraw_from_pool` (3437), `delegated_withdraw` (4139), `trigger_auto_claim` (7616) |
| `closed` | `close_cancelled_stream` (5789) |
| `rate_upd` | `update_rate` (6406) |
| `resumed` | `bulk_resume_streams_as_admin` (6974) |
| `cancelled` | `bulk_cancel_streams` (8266) |
| `health` | `keeper_cancel` (6676), `bulk_cancel_streams` (8268) via `maybe_emit_health_changed` (`storage.rs:742`) |
| `recp_upd` | emitted in `accept_recipient_update` (3677), the completion half of the two-phase rotation started by `update_recipient` (3599) |

### F7 — Legacy-table function names have drifted — ⚠️ (legacy table only)

- `resume_global_pause` → actual `global_resume` (`lib.rs:7035`).
- `set_decommissioned` → actual `set_stream_decommissioned` (`lib.rs:5506`).
- Legacy table also omits post-refactor emitters entirely (`delegated_withdraw`,
  `withdraw_from_pool`, `bulk_*`, `update_rate`, `close_cancelled_stream`,
  `register_stream_template`, `release_reservation`, `accept_stream_offer`).
- Recommendation stands from the previous analysis: **retire the legacy table** and keep only
  the canonical one.

### F8 — Dangling test reference in `docs/events.md` — ⚠️

`docs/events.md:799` claims "The test in `tests/event_schema_consistency.rs` cross-checks
every event struct against the documented shapes below." **That file does not exist** in the
repo (`ls tests/` shows no such file). Either add the test or remove the claim.

## Status of the previous analysis' identified discrepancy

| Previous claim | Current status (re-verified 2026-07-29 @ `046ad40`) |
| --- | --- |
| `set_admin` used `Symbol::new(&env, "AdminUpdated")` — ⚠️ MISALIGNMENT | **Fixed in code**: `symbol_short!("AdminUpd")` at `lib.rs:4455–4456`. Topic now matches the canonical table. Remaining drift is doc-side (§9), tracked as F4. |
| All other rows "✅ Aligned" at cited lines 474/794/839/… | **Citations were stale by ~1,175–6,000 lines** (e.g. `persist_new_stream` 474 → 1649; emission moved into `events.rs` helpers). Verdicts re-derived from scratch above: 38 of 40 emission shapes align; 2 shape-level mismatches found (F1, F2), plus attribution/doc issues F3–F8. |

## Cross-document staleness pattern (maintainer flag)

This is **not** an isolated failure. Several analysis documents in this repo were clearly
generated together at one point in time and never refreshed:

- This file cited `persist_new_stream` at **line 474**; it is now at **1,649**.
- `docs/PROTOCOL_NARRATIVE_VS_CODE_ALIGNMENT.md` cites `cancel_stream_as_admin` /
  `pause_stream_as_admin` / `resume_stream_as_admin` at `lib.rs:2033/2063/2095`; they are now
  at **6459 / 6773 / 6857** — the same order-of-magnitude (~4,500-line) drift.
- `docs/stream-id-monotonicity-uniqueness.md` references the same `persist_new_stream`
  flow without line numbers (safer, but still undated).

**Recommendation to maintainers:** treat every undated analysis doc as suspect; require a
"Last performed / commit" stamp (like the header of this file) on all generated analyses, and
gate merges touching `contracts/stream/src/lib.rs` or `docs/events.md` on a re-run of
`script/validate-doc-alignment.py` (which already checks event symbols against `docs/events.md`)
plus regeneration of this file.

## Regenerating this analysis

```bash
# 1. All direct publish sites (catches multi-line calls that `events().publish` misses):
grep -n "\.publish(" contracts/stream/src/lib.rs

# 2. Helper-mediated emissions:
grep -n "events::emit_" contracts/stream/src/lib.rs contracts/stream/src/storage.rs
grep -n "\.publish(" contracts/stream/src/events.rs

# 3. Verify "not emitted" claims:
grep -rn "<topic_symbol>" contracts/ --include="*.rs"

# 4. Existing CI helper for symbol-vs-doc coverage:
python3 script/validate-doc-alignment.py
```

Then update the header stamp (date + `git rev-parse HEAD`) and re-derive every row.
**Never patch a single stale line number** — re-run the enumeration, because function
boundaries and the emission architecture itself can change (as the `events.rs` refactor did).

## Edge Cases Verified (re-checked against current code)

| Event       | Emitted when                                    | Not emitted when                                        |
| ----------- | ----------------------------------------------- | ------------------------------------------------------- |
| `created`   | After successful persistence + token transfer (`persist_new_stream` 1649, `…_skip_index` 1749, pooled 2315, offer-accept 8465) | Any validation/auth/transfer failure |
| `withdrew`  | `withdraw`/`withdraw_from_pool`/`delegated_withdraw` when `withdrawable > 0`; `bulk_cancel_streams` pays accrued portion | `withdrawable == 0` (idempotent no-op) |
| `wdraw_to`  | `withdraw_to`/`batch_withdraw_to`/`batch_withdraw`/`trigger_auto_claim` when `withdrawable > 0` | `withdrawable == 0` |
| `completed` | Final drain of an Active stream on any withdraw path | Cancelled streams; partial withdrawals |
| `paused`    | Active stream paused (sender 2939 / admin 6773) | Already Paused/Completed/Cancelled |
| `resumed`   | Paused stream resumed (3018 / 6857 / bulk 6920) | Active/Completed/Cancelled |
| `cancelled` | Active/Paused cancelled (`cancel_stream_internal` 6306; bulk 8155) | Already Cancelled/Completed |
| `closed`    | Completed (5693) or Cancelled (5762) stream archived | Non-terminal streams |
| `rate_upd`  | Rate increased/changed via `update_rate_per_second` 4714 or `update_rate` 6354 | Validation failure / terminal status |
| `rate_dec`  | Safe checkpointed decrease via 4862             | Validation failure / terminal status                    |
| `rate_cap`  | Requested rate exceeds governance cap (4760)    | Rate within cap                                         |
| `end_shrt`  | Successful shorten (5182)                       | Failure paths                                           |
| `end_ext`   | Successful extend (5304)                        | Failure paths                                           |
| `top_up`    | Successful top-up (5411)                        | Failure paths                                           |
| `health`    | Only on funded↔underfunded **transition** (`storage.rs:739`) | No transition |
| `AdminUpd`  | Admin rotated via `set_admin` (4440)            | Auth failure                                            |

## Authorization Matrix (function lines current as of `046ad40`)

| Event       | Required authorization        | Admin override                     |
| ----------- | ----------------------------- | ---------------------------------- |
| `created`   | Sender                        | No (global pauses via admin)       |
| `withdrew`  | Recipient (or delegatee with nonce for `delegated_withdraw` 4004) | No |
| `wdraw_to`  | Recipient (or permissionless `trigger_auto_claim` 7507 when configured) | No |
| `completed` | Recipient (via withdraw path) | No                                 |
| `paused`    | Sender (2939)                 | Yes (`pause_stream_as_admin` 6773) |
| `resumed`   | Sender (3018)                 | Yes (`resume_stream_as_admin` 6857, `bulk_resume_streams_as_admin` 6920) |
| `cancelled` | Sender (3106)                 | Yes (`cancel_stream_as_admin` 6459; `bulk_cancel_streams` 8155; `keeper_cancel` 6568 after grace period) |
| `closed`    | None (permissionless)         | N/A                                |
| `rate_upd`  | Sender or admin (`update_rate` 6354); sender (`update_rate_per_second` 4714) | Partial |
| `rate_dec`  | Sender (4862)                 | No                                 |
| `end_shrt` / `end_ext` / `top_up` | Sender     | No                                 |
| `AdminUpd`  | Current admin (4440)          | No                                 |
| `upgraded`  | Admin (8794)                  | No                                 |

## Residual Risks

1. **F1 (`res_rel` widths)** — active mis-parsing risk for indexers built against the doc. Fix doc or code (version-gated).
2. **F3 (`batch_withdraw` topic)** — indexers filtering `withdrew` will silently miss batch withdrawals. Doc fix only (code behavior is intentional and consistent with `WithdrawalTo`).
3. **F5 (`sndr_xfr` phantom)** — integrators may build against an event that never fires.
4. **Breaking-change risk** if `AdminUpd` naming ever changes again — the previous analysis'
   migration caveat still applies; the topic has been stable since the v3 admin-event fix.
5. **Documentation drift** — mitigations: CI run of `script/validate-doc-alignment.py`, the
   date/commit stamp convention adopted by this file, and retiring the legacy table (F7).

## Definition of Done Checklist

- [x] All event emissions in current code re-enumerated (12 direct + 28 helpers + 1 storage helper, commit `046ad40`)
- [x] Every row checked against the **current canonical** `docs/events.md` table; legacy table used as cross-check only, with disagreements reported (F3, F4, F7)
- [x] Fresh "Aligned"/"Not Aligned" verdict per row — none carried forward from the stale snapshot
- [x] Every line-number citation re-derived from the current `lib.rs` / `events.rs` / `storage.rs`
- [x] Date + commit-hash stamp added (header and footer)
- [x] Root-cause and cross-document staleness pattern flagged to maintainers
- [x] Regeneration procedure documented to prevent silent-staleness recurrence
- [ ] F1–F8 fixes implemented in `docs/events.md` / code (separate follow-up; this file is the analysis deliverable)

## Conclusion

Re-verified at commit `046ad40` (2026-07-29): of 41 distinct emission sites (12 direct
`publish` calls in `lib.rs`, 28 `events.rs` helpers, 1 `storage.rs` helper), **topic and
payload shapes align with the canonical `docs/events.md` table at 38 sites**. The re-analysis
surfaces two shape-level mismatches the stale snapshot could not see — `res_rel` payload
widths (F1) and the undocumented-but-emitted `tmpl_def` (F2) — plus a wrong function→topic
attribution for `batch_withdraw` (F3), two doc-internal contradictions where the code matches
the canonical table (F4), a documented-but-never-emitted `sndr_xfr` event (F5), incomplete
emitter lists (F6), legacy-table function-name drift (F7), and a dangling test reference (F8).
The previous analysis' one code-level discrepancy (`set_admin` topic construction) is
confirmed **fixed in code**; its documentation half remains open in `docs/events.md` §9/§15.

---

_Last analysis run: **2026-07-29** · commit **046ad40934e86f908c74c17968940d02baa4336a** · regenerate per §Regenerating this analysis whenever `contracts/stream/src/{lib,events,storage}.rs` or `docs/events.md` change._
