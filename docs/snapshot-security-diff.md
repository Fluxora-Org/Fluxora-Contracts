# Snapshot Security Diff

`script/check_snapshot_diff.py` classifies whether a diff between two contract
snapshot JSON files contains **security-relevant field changes** that require
mandatory extra reviewer scrutiny before a PR is merged.

> **CI status:** This tool is **wired into CI** as a hard gate in the `test`
> job of `.github/workflows/ci.yml`. It runs on every `pull_request` and `push`
> event, comparing the PR's changed snapshot files against the base branch.
> A non-zero exit code blocks merge. For manual usage, see [CLI usage](#cli-usage).

---

## Table of contents

1. [Purpose and scope](#purpose-and-scope)
2. [SECURITY_FIELDS — what is classified and why](#security_fields--what-is-classified-and-why)
3. [is_security_relevant — path-matching algorithm](#is_security_relevant--path-matching-algorithm)
4. [CLI usage](#cli-usage)
5. [Exit-code contract](#exit-code-contract)
6. [CI integration](#ci-integration)
7. [Reviewer workflow](#reviewer-workflow)
8. [Output examples](#output-examples)
9. [Security guarantees](#security-guarantees)
10. [Relationship to other docs](#relationship-to-other-docs)

---

## Purpose and scope

Snapshot tests capture the complete externally-observable state of the
FluxoraStream contract after each operation. When contract code changes,
snapshot files change too — and most of the time that is routine (a status
transition, an updated stream count, a new timestamp).

A small subset of field changes are **security-relevant**: they touch
authorization paths, token identity, error codes, or storage layout. Those
changes warrant a deeper look because a mistake there can lead to fund loss,
privilege escalation, or a locked contract. This script makes it cheap to
detect them.

**What this script does:**

- Lists `contracts/stream/test_snapshots/*.json` files changed between two git refs.
- Reads both the old and new version of each file (from git history or the working tree).
- Recursively diffs the old and new JSON structures to produce dotted-key paths.
- Classifies each changed key against `SECURITY_FIELDS`.
- Reports findings and exits with a machine-readable code.

**What this script does not do:**

- It does not validate contract logic or cryptographic correctness.
- It does not replace a full security audit.
- It is not a substitute for human review of every snapshot diff.
- It does not produce JSON output or support `--quiet` mode (these are future enhancements).

---

## SECURITY_FIELDS — what is classified and why

`SECURITY_FIELDS` is a `set` of sentinel strings defined at the top of
`script/check_snapshot_diff.py`. A snapshot field is flagged when **any part**
of its dotted key path (split on `.` and `[]`) matches a sentinel exactly.
The current sentinels, grouped by category:

### Authorization / authentication

| Sentinel | Rationale |
|---|---|
| `auth` | Top-level or nested authorization envelope. Changes indicate different signer sets. |
| `auths` | Plural authorization list. Adding or removing signers changes who can act. |
| `require_auth` | Boolean authorization flag. Flipping this changes whether auth is enforced. |
| `signatures` | Cryptographic signature array. New or modified signatures may indicate different signers. |

### Events and data

| Sentinel | Rationale |
|---|---|
| `events` | Event array. New events, removed events, or changed event payloads alter the externally-observable audit trail. |
| `topic` | Event topic (the event discriminator). Changing a topic alters the event's semantic meaning. |
| `topics` | Plural topic array. Same rationale as `topic`. |
| `data` | Event data payload. Changes to data fields alter what information the event carries. |

### Error classification

| Sentinel | Rationale |
|---|---|
| `error` | Error variant name. Changing which error is returned alters the contract's failure semantics. |
| `error_code` | Numeric error discriminant. A changed discriminant silently alters the error's identity. |
| `ContractError` | The error enum type itself. Any change here signals a new or modified error variant. |

### Storage layout

| Sentinel | Rationale |
|---|---|
| `storage` | Top-level storage namespace. Changes here indicate a structural shift in persisted state. |
| `state` | State field within storage. Changing state values directly alters contract behavior. |
| `DataKey` | Storage key discriminant. A shift here silently corrupts persistent storage reads. See §6 of `docs/maintainer-security-checklist.md`. |

---

## is_security_relevant — path-matching algorithm

```python
def is_security_relevant(path):
    parts = path.replace('[', '.').replace(']', '').split('.')
    for part in parts:
        if part in SECURITY_FIELDS:
            return True
    return False
```

The algorithm normalises a dotted key path by replacing `[` and `]` with `.`,
then splitting on `.`. Each resulting segment is checked for **exact membership**
in `SECURITY_FIELDS`. If any segment matches, the path is security-relevant.

### Worked example

Given the following diff between `base.json` and `head.json`:

```
base.json                              head.json
─────────────────────────────────────  ─────────────────────────────────────
config.admin      = "GADMIN111"        config.admin      = "GADMIN999"   ← changed
config.token      = "GTOKEN111"        config.token      = "GTOKEN111"   (same)
streams[0].status = "Active"           streams[0].status = "Completed"   ← changed
streams[0].rate_per_second = 100       streams[0].rate_per_second = 100  (same)
next_stream_id    = 1                  next_stream_id    = 2             ← changed
events[0].topic   = "created"          events[0].topic   = "created"    (same)
```

Classification walkthrough:

| Dotted key | Split segments | Sentinel hit | Flagged? |
|---|---|---|---|
| `config.admin` | `["config", "admin"]` | `"admin"` ∈ SECURITY_FIELDS | ✅ |
| `streams[0].status` | `["streams", "0", "status"]` | none | ❌ |
| `next_stream_id` | `["next_stream_id"]` | none | ❌ |
| `events[0].topic` | `["events", "0", "topic"]` | `"events"` ∈ SECURITY_FIELDS | ✅ |

Result: **2 security-relevant changes** (`config.admin`, `events[0].topic`).
Exit code **1**.

---

## CLI usage

```
python script/check_snapshot_diff.py --base <REF> [--head <REF>]
```

### Arguments

| Argument | Default | Description |
|---|---|---|
| `--base REF` | `HEAD` | Git ref for the **before** state (commit SHA, branch name, etc.). |
| `--head REF` | `None` (working tree) | Git ref for the **after** state. When omitted, compares against the current working tree. |

### Examples

```bash
# Compare snapshot changes between main and the current working tree
python script/check_snapshot_diff.py --base origin/main

# Compare two specific commits
python script/check_snapshot_diff.py --base abc1234 --head def5678

# Compare against the previous commit (common in push-based CI)
python script/check_snapshot_diff.py --base HEAD~1

# Use from a PR context (base branch ref)
python script/check_snapshot_diff.py --base origin/main --head HEAD
```

---

## Exit-code contract

| Code | Meaning |
|---|---|
| `0` | No security-relevant field changes detected. Normal review applies. |
| `1` | One or more security-relevant field changes detected. **Mandatory extra review required.** |
| Non-zero (other) | Usage error (e.g. git failure, missing repo). |

The exit code is stable and machine-readable. The CI pipeline gates merge
on `$? -eq 0` — a non-zero exit blocks the PR.

---

## CI integration

The tool runs as a **hard gate** (no `continue-on-error`) in the `test` job
of `.github/workflows/ci.yml`:

```yaml
- name: Snapshot security-field diff gate
  if: github.event_name == 'pull_request' || github.event_name == 'push'
  run: |
    if [ "${{ github.event_name }}" = "pull_request" ]; then
      BASE="origin/${{ github.base_ref }}"
    else
      BASE="HEAD~1"
    fi
    python3 script/check_snapshot_diff.py --base "${BASE}"
```

**Behaviour per event type:**

| Event | Base ref | What is compared |
|---|---|---|
| `pull_request` | `origin/${{ github.base_ref }}` | PR branch snapshot files vs. target branch |
| `push` | `HEAD~1` | Pushed commit vs. its parent |
| `workflow_dispatch` | Skipped | Tool does not run |

**Merge blocking:** The step has no `continue-on-error`. A non-zero exit
from `check_snapshot_diff.py` fails the `test` job, which blocks the
`build` and deployment jobs that depend on it.

---

## Reviewer workflow

### When a PR changes snapshot files

1. **Identify changed snapshots** in the PR diff:
   ```bash
   git diff --name-only origin/main | grep 'test_snapshots'
   ```

2. **The CI gate runs automatically** and reports its findings in the
   workflow log. Check the `Snapshot security-field diff gate` step.

3. **If the gate passed (exit 0):** No flagged fields. Proceed with standard
   snapshot review — confirm that changed fields reflect the intended
   behaviour described in the PR description.

4. **If the gate failed (exit 1):** Flagged fields present. Apply the
   mandatory extra review steps below before approving.

### Mandatory extra review steps when exit code is 1

When the script flags one or more security-relevant changes, the reviewer
**must** complete all applicable steps before approving:

- [ ] **Auth change** (`auth`, `auths`, `require_auth`, `signatures`):
  Confirm the new authorization envelope matches the intended signer set.
  Verify that `require_auth()` was called correctly. Check that no
  unintended signers were added or removed.

- [ ] **Event change** (`events`, `topic`, `topics`, `data`): Confirm the
  new event payload matches the contract behaviour described in the PR.
  A changed `topic` may indicate a different event type was emitted.

- [ ] **Error change** (`error`, `error_code`, `ContractError`): Verify the
  new error variant/discriminant matches the intended change. Check that
  existing callers are not broken by a discriminant reordering. See
  `docs/error.md` for the authoritative discriminant table.

- [ ] **Storage change** (`storage`, `state`, `DataKey`): This is a critical
  finding. Confirm that no existing `DataKey` variants were reordered and
  that any new variants were appended at the end. A `DataKey` discriminant
  shift silently corrupts persistent storage. See §6 of
  `docs/maintainer-security-checklist.md`.

Record your review in the PR comment thread. Approval without a documented
review of flagged fields is insufficient.

---

## Output examples

### No security-relevant changes

```
No snapshot JSON files changed.
```

or (when snapshot files changed but none contain security-relevant diffs):

```
[INFO] Changes in contracts/stream/test_snapshots/test_create_stream.1.json (none are security-relevant)

No security-relevant snapshot changes detected.
```

### Security-relevant changes detected

```
[WARNING] Security-relevant fields changed in: contracts/stream/test_snapshots/test_create_stream.1.json
  - events[0].data.topic
  - config.admin

Mandatory extra review required due to security-relevant snapshot changes.
```

---

## Security guarantees

The following guarantees are validated by the test suite in
`tests/test_check_snapshot_diff.py`:

| Guarantee | Validated by |
|---|---|
| Only files under `/test_snapshots/*.json` trigger analysis | `TestGetChangedFiles` |
| Every `SECURITY_FIELDS` member is individually tested as relevant | `TestIsSecurityRelevant` |
| Nested and list-indexed paths are correctly classified | `TestGetDiffPaths`, `TestIsSecurityRelevant` |
| Malformed JSON for either side is treated as `{}` (no crash) | `TestMain.test_malformed_old_json_treated_as_empty` |
| Missing file on either side returns `{}` (no crash) | `TestMain.test_none_old_and_new_content` |
| Exit 1 when any file among multiple changed files has security diffs | `TestMain.test_one_of_multiple_files_triggers_exit_1` |
| Real git repos produce correct exit codes end-to-end | `TestMainEndToEndRealGitRepo` |

Run the test suite:

```bash
pytest tests/test_check_snapshot_diff.py -v
```

---

## Relationship to other docs

| Document | Relationship |
|---|---|
| [`docs/snapshot-workflow-quick-reference.md`](snapshot-workflow-quick-reference.md) | Day-to-day snapshot update workflow. Run `check_snapshot_diff.py` after `SOROBAN_SNAPSHOT_UPDATE=1` and before committing. |
| [`docs/maintainer-security-checklist.md`](maintainer-security-checklist.md) | Full pre-merge security checklist. §2 (auth boundaries), §6 (DataKey safety), and §8 (pause state) map directly to flagged field categories. |
| [`docs/error.md`](error.md) | Authoritative `ContractError` discriminant table. Error changes flagged by this tool should be cross-checked against this table. |
| `tests/test_check_snapshot_diff.py` | Unit + real-git-repo end-to-end tests for the diff gate. 99% line coverage. |
