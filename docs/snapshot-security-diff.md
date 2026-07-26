# Snapshot Security Diff

`script/check_snapshot_diff.py` classifies whether a diff between two contract
snapshot JSON files contains **security-relevant field changes** that require
mandatory extra reviewer scrutiny before a PR is merged.

> **CI status:** This tool is **not yet wired into CI**. The companion
> CI-wiring issue must land before these checks are enforced automatically.
> Until then, maintainers must run this script manually during PR review
> whenever snapshot files change. See [Reviewer workflow](#reviewer-workflow).

---

## Table of contents

1. [Purpose and scope](#purpose-and-scope)
2. [SECURITY_FIELDS — what is classified and why](#security_fields--what-is-classified-and-why)
3. [is_security_relevant — path-matching algorithm](#is_security_relevant--path-matching-algorithm)
4. [CLI usage](#cli-usage)
5. [Exit-code contract](#exit-code-contract)
6. [Reviewer workflow](#reviewer-workflow)
7. [Output formats](#output-formats)
8. [Relationship to other docs](#relationship-to-other-docs)

---

## Purpose and scope

Snapshot tests capture the complete externally-observable state of the
FluxoraStream contract after each operation. When contract code changes,
snapshot files change too — and most of the time that is routine (a status
transition, an updated stream count, a new timestamp).

A small subset of field changes are **security-relevant**: they touch
authorization paths, token identity, rate caps, or pause state. Those changes
warrant a deeper look because a mistake there can lead to fund loss, privilege
escalation, or a locked contract. This script makes it cheap to detect them.

**What this script does:**

- Loads two snapshot JSON files (`--base` for before, `--head` for after).
- Recursively flattens both into dotted-key paths (`config.admin`,
  `streams.0.rate_per_second`, …).
- Computes added, removed, and changed keys.
- Classifies each changed key against `SECURITY_FIELDS`.
- Reports findings and exits with a machine-readable code.

**What this script does not do:**

- It does not validate contract logic or cryptographic correctness.
- It does not replace a full security audit.
- It is not a substitute for human review of every snapshot diff.

---

## SECURITY_FIELDS — what is classified and why

`SECURITY_FIELDS` is a `frozenset[str]` of lowercase sentinel strings defined
at the top of `script/check_snapshot_diff.py`. A snapshot field is flagged
when its dotted key path **equals or contains** any sentinel as a substring
(case-insensitive). The current sentinels, grouped by category:

### Authorization / admin

| Sentinel | Rationale |
|---|---|
| `admin` | The contract admin can pause the protocol, cancel any stream, rotate keys, and set rate caps. Any change to the admin address is a critical privilege transfer. |
| `admin_address` | Composite form of the same field that may appear in factory or governance snapshots. |
| `require_auth` | If this sentinel ever appears in a snapshot value it signals a stored authorization policy object; changes must be reviewed. |

### Token identity and trust

| Sentinel | Rationale |
|---|---|
| `token` | The token contract address is trusted for all fund custody. Swapping it redirects every future transfer. See `docs/token-assumptions.md`. |
| `token_address` | Alternate key used by factory and indexer-layer snapshots. |
| `token_contract` | Used in governance and cross-contract call contexts. |

### Rate and accrual limits

| Sentinel | Rationale |
|---|---|
| `rate_per_second` | Per-stream accrual rate. Changing it after stream creation affects how much a recipient can withdraw. |
| `max_rate_per_second` | Global cap enforced at stream creation. Raising it silently allows higher-than-reviewed rates on new streams. |
| `deposit_amount` | Total funds locked in a stream. An unexpected increase or decrease signals a top-up or an under-refund. |

### Recipient and delegation

| Sentinel | Rationale |
|---|---|
| `recipient` | The address entitled to withdraw funds. Any rotation must go through the `update_recipient` / `accept_recipient_update` flow. |
| `pending_recipient_update` | Intermediate state of a recipient rotation. Appearing unexpectedly may indicate an unauthorized rotation attempt. |
| `delegated_nonce` | Replay-protection counter for `delegated_withdraw`. An unexpected reset enables replay of a prior signed withdrawal. |
| `nonce` | Generic nonce field; catches composite keys like `streams.0.nonce`. |

### Pause and emergency state

| Sentinel | Rationale |
|---|---|
| `paused` | Matches `global_emergency_paused`, `creation_paused`, and any stream-level pause field. |
| `emergency` | Matches `global_emergency_paused` and any future emergency sentinel. |
| `pause_state` | Enum field used in governance / factory snapshots. |
| `creation_paused` | Blocks new stream creation when `true`. |
| `global_emergency_paused` | Blocks all user-facing mutations when `true`. |

### Storage layout sentinels

| Sentinel | Rationale |
|---|---|
| `data_key` | A change here signals a `DataKey` discriminant shift, which silently corrupts persistent storage. See §6 of `docs/maintainer-security-checklist.md`. |
| `contract_version` | A version bump must accompany every breaking change. An unexpected bump (or absence of one) is a red flag. |

---

## is_security_relevant — path-matching algorithm

```python
def is_security_relevant(field_key: str) -> bool:
    lowered = field_key.lower()
    for sentinel in SECURITY_FIELDS:
        if lowered == sentinel:        # 1. exact match
            return True
        if sentinel in lowered:        # 2. substring match
            return True
    return False
```

The algorithm has two steps, applied to the **lowercased** field key:

1. **Exact match** — the key equals a sentinel verbatim.
2. **Substring match** — the sentinel appears anywhere inside the dotted key.
   This catches nested paths without enumerating every possible prefix.

### Worked example

Given the following diff between `base.json` and `head.json`:

```
base.json                          head.json
─────────────────────────────────  ──────────────────────────────────
config.admin      = "GADMIN111"    config.admin      = "GADMIN999"   ← changed
config.token      = "GTOKEN111"    config.token      = "GTOKEN111"   (same)
streams.0.status  = "Active"       streams.0.status  = "Completed"   ← changed
streams.0.rate_per_second = 100    streams.0.rate_per_second = 100   (same)
next_stream_id    = 1              next_stream_id    = 2             ← changed
```

Classification walkthrough:

| Dotted key | Lowered | Sentinel hit | Flagged? |
|---|---|---|---|
| `config.admin` | `config.admin` | `"admin"` is a substring → yes | ✅ |
| `streams.0.status` | `streams.0.status` | no sentinel matches | ❌ |
| `next_stream_id` | `next_stream_id` | no sentinel matches | ❌ |

Result: **1 security-relevant change** (`config.admin`). Exit code **1**.

---

## CLI usage

```
python script/check_snapshot_diff.py --base <FILE> --head <FILE> [--quiet] [--output-format {human,json}]
```

### Required arguments

| Argument | Description |
|---|---|
| `--base FILE` | Path to the **before** snapshot JSON (e.g. the file on `main`). |
| `--head FILE` | Path to the **after** snapshot JSON (e.g. the file in the PR branch). |

### Optional arguments

| Argument | Default | Description |
|---|---|---|
| `--quiet` | off | Suppress all stdout. Only the exit code is meaningful. Errors still go to stderr. |
| `--output-format` | `human` | `human` for readable text; `json` for machine-parseable output. |

### Examples

```bash
# Compare two snapshot files, human output
python script/check_snapshot_diff.py \
  --base contracts/stream/test_snapshots/test/test_create_stream.1.json \
  --head /tmp/head_snapshot.json

# Machine-parseable JSON output (useful for scripting)
python script/check_snapshot_diff.py \
  --base base.json --head head.json \
  --output-format json

# Silent check — only the exit code matters
python script/check_snapshot_diff.py \
  --base base.json --head head.json --quiet
echo "Exit: $?"
```

---

## Exit-code contract

| Code | Meaning |
|---|---|
| `0` | No security-relevant field changes detected. Normal review applies. |
| `1` | One or more security-relevant field changes detected. **Mandatory extra review required.** |
| `2` | Usage error: missing file, unreadable file, invalid JSON, or wrong JSON type at top level. |

The exit code is stable and machine-readable. Scripts and future CI jobs should
test for `$? -eq 1` to gate on security-relevant diffs.

---

## Reviewer workflow

### When a PR changes snapshot files

1. **Identify changed snapshots** in the PR diff:
   ```bash
   git diff --name-only origin/main | grep 'test_snapshots'
   ```

2. **Extract the base version** of each changed snapshot from `main`:
   ```bash
   git show origin/main:contracts/stream/test_snapshots/test/test_NAME.1.json \
     > /tmp/base_snapshot.json
   ```

3. **Run the security diff check** against the head version:
   ```bash
   python script/check_snapshot_diff.py \
     --base /tmp/base_snapshot.json \
     --head contracts/stream/test_snapshots/test/test_NAME.1.json
   ```

4. **Act on the exit code:**

   - **Exit 0** — No flagged fields. Proceed with standard snapshot review:
     confirm that changed fields reflect the intended behaviour described in
     the PR description.

   - **Exit 1** — Flagged fields present. Apply the mandatory extra review
     steps below before approving.

   - **Exit 2** — Script error. Fix the invocation or check the file paths.

### Mandatory extra review steps when exit code is 1

When the script flags one or more security-relevant changes, the reviewer
**must** complete all applicable steps before approving:

- [ ] **Admin change** (`admin`, `admin_address`): Confirm the new admin
  address matches the intended recipient documented in the PR. Verify the
  `set_admin` entrypoint was called with the current admin's auth. Cross-check
  §2.2 of `docs/maintainer-security-checklist.md`.

- [ ] **Token change** (`token`, `token_address`, `token_contract`): Confirm
  the new token address is the intended USDC contract on the target network.
  Review `docs/token-assumptions.md` for trust-model implications.

- [ ] **Rate change** (`rate_per_second`, `max_rate_per_second`): Verify the
  new rate value is within the reviewed budget. For `max_rate_per_second`,
  confirm the admin auth was correctly supplied. Check that the accrual
  formula still holds at the new rate.

- [ ] **Deposit change** (`deposit_amount`): Confirm the change matches either
  a `top_up_stream` call (increase) or a `cancel_stream` / `shorten_stream_end_time`
  refund (decrease). Verify no funds are unaccounted for.

- [ ] **Recipient change** (`recipient`, `pending_recipient_update`): Confirm
  the rotation followed the two-step `update_recipient` → `accept_recipient_update`
  flow. Check that the old recipient's withdrawal rights were not stripped
  mid-stream.

- [ ] **Nonce change** (`nonce`, `delegated_nonce`): Confirm the nonce
  incremented (never decreased or reset to zero). A reset enables replay of a
  prior signed delegation.

- [ ] **Pause state change** (`paused`, `emergency`, `pause_state`,
  `creation_paused`, `global_emergency_paused`): Confirm the intended pause
  semantics. Verify the admin auth was supplied. Review §8 of
  `docs/maintainer-security-checklist.md` for what each pause state blocks.

- [ ] **Contract version change** (`contract_version`): Cross-check that the
  bump is accompanied by a `CHANGELOG.md` entry and, if a breaking change, the
  `DataKey` discriminant table is current. See §4 of
  `docs/maintainer-security-checklist.md`.

- [ ] **Storage key change** (`data_key`): This is a critical finding. Confirm
  that no existing `DataKey` variants were reordered and that any new variants
  were appended at the end. See §6 of `docs/maintainer-security-checklist.md`.

Record your review in the PR comment thread. Approval without a documented
review of flagged fields is insufficient.

---

## Output formats

### Human (default)

When no security-relevant changes are detected:

```
check_snapshot_diff: no security-relevant changes detected.
```

When changes are detected:

```
check_snapshot_diff: 2 security-relevant change(s) detected.
  base: contracts/stream/test_snapshots/test/test_create_stream.1.json
  head: /tmp/head_snapshot.json

  Mandatory extra review required (see docs/snapshot-security-diff.md).

  [CHANGED] config.admin  'GADMIN111' → 'GADMIN999'
  [CHANGED] config.max_rate_per_second  1000 → 9999
```

### JSON (`--output-format json`)

```json
{
  "flagged": true,
  "base": "base.json",
  "head": "head.json",
  "security_relevant_changes": [
    {
      "key": "config.admin",
      "change_type": "changed",
      "base": "GADMIN111",
      "head": "GADMIN999"
    },
    {
      "key": "config.max_rate_per_second",
      "change_type": "changed",
      "base": 1000,
      "head": 9999
    }
  ]
}
```

The `flagged` boolean mirrors the exit code: `true` = exit 1, `false` = exit 0.
`change_type` is one of `"added"`, `"removed"`, or `"changed"`.
For `"added"`, `base` is `null`. For `"removed"`, `head` is `null`.

---

## Relationship to other docs

| Document | Relationship |
|---|---|
| [`docs/snapshot-workflow-quick-reference.md`](snapshot-workflow-quick-reference.md) | Day-to-day snapshot update workflow. Run `check_snapshot_diff.py` after `SOROBAN_SNAPSHOT_UPDATE=1` and before committing. |
| [`docs/maintainer-security-checklist.md`](maintainer-security-checklist.md) | Full pre-merge security checklist. §2 (auth boundaries), §6 (DataKey safety), and §8 (pause state) map directly to flagged field categories. |
| [`docs/security.md`](security.md) | CEI ordering, token trust model, and admin auth paths — the threat model that motivates each `SECURITY_FIELDS` entry. |
| [`docs/snapshot-tests.md`](snapshot-tests.md) | What snapshot tests capture and how they are structured. |
| `tests/test_check_snapshot_diff.py` | 144-test suite for `check_snapshot_diff.py`; 99 % line coverage. Run with `pytest tests/test_check_snapshot_diff.py -v`. |
