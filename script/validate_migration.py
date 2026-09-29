#!/usr/bin/env python3
"""Validate docs/MIGRATION.md claims against the actual v1 contract ABI.

The migration document (docs/MIGRATION.md) describes how the v1 rewrite changed
the public entrypoint surface: which functions were renamed, which were removed,
and how signatures evolved.  Without an automated check the document drifts from
the code — a reader following the "described path" ends up with calls that do
not exist on the deployed contract.

This script closes that gap by parsing the documented claims from
``MIGRATION.md`` and cross-checking them against the frozen ABI snapshot in
``contracts/stream/abi/fluxora_stream.json``.  A mismatch exits with code 1 so
CI blocks the PR.

Checks performed (each maps to a section of MIGRATION.md):

  §3  – Removed entrypoints: every function in the curated removed list must
        NOT appear in the v1 ABI.
  §4  – Renamed entrypoints: every ``v1`` column value from the doc table must
        exist in the ABI, and every ``old`` column value must NOT.
  §4  – Signature changes: ``create_stream`` must carry a ``token`` parameter
        and the three capability flags; ``withdraw`` must accept
        ``Option<i128>`` as its second argument.
  §3  – Entrypoint count: the ``16`` core count claimed in the doc must match
        the number of non-delegation entrypoints in the ABI.

Exit codes:  0 = all checks passed, 1 = one or more checks failed.
"""

import json
import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
ABI_PATH = REPO_ROOT / "contracts" / "stream" / "abi" / "fluxora_stream.json"
MIGRATION_PATH = REPO_ROOT / "docs" / "MIGRATION.md"

# --- Entrypoint classification ----------------------------------------------

# v1 core (non-delegation) entrypoints — the 21 the migration document counts.
# Everything else is a delegation variant.
DELEGATION_PREFIXES = ("delegate_", "grant_delegate", "revoke_delegate")

CORE_ENTRYPOINT_COUNT = 21

# --- Removed entrypoints extracted from MIGRATION.md §3 ---------------------
# Every name that appears in §3 as "deliberately removed" must NOT exist in v1.
# This list is curated from the prose in §3 (grouped by reason).  It is kept
# here as a constant rather than parsed from prose because §3 mentions several
# v1 entrypoints (e.g. ``extend_stream_ttl``, ``top_up``, ``withdraw``) in the
# context of explaining what v1 *keeps*, and a naive backtick extraction would
# wrongly classify them as removed.
REMOVED_ENTRYPOINTS = {
    # Contradicts §6 (no admin, no upgradeability, no fees, no global pause)
    "init",
    "set_admin",
    "upgrade",
    "version",
    "pause_protocol",
    "resume_protocol",
    "global_resume",
    "set_global_emergency_paused",
    "get_global_emergency_paused",
    "set_contract_paused",
    "is_paused",
    "cancel_stream_as_admin",
    "pause_stream_as_admin",
    "resume_stream_as_admin",
    "bulk_resume_streams_as_admin",
    "set_stream_decommissioned",
    "sweep_excess",
    "get_protocol_fees_accrued",
    "get_keeper_fee_split",
    "set_max_rate_per_second",
    # Contradicts §2.3 (no on-chain stream discovery)
    "get_recipient_streams",
    "get_recipient_streams_paginated",
    "get_recipient_stream_count",
    "get_streams_by_id_range",
    "get_sender_portfolio_health",
    "get_paused_stream_count",
    "get_total_liabilities",
    "get_factory_streams_paginated",
    # Out of v1 scope, deferred to the SDK or a later version
    "create_stream_relative",
    "create_streams_relative",
    "create_streams",
    "create_streams_partial",
    "reserve_stream_ids",
    "get_id_reservation",
    "release_id_reservation",
    "reclaim_expired_id_reservation",
    "clone_stream",
    "create_stream_from_template",
    "create_pooled_stream",
    "withdraw_from_pool",
    "create_stream_offer",
    "set_auto_claim",
    "set_auto_renew",
    "renew_stream",
    "delegate_recipient_share",
    "witnessed_cancel_stream",
    "transfer_claim_ownership",
    "get_stream_metadata",
    "get_stream_memo",
    # Contradicts the immutability guarantee (§2.2)
    "update_rate",
    "update_rate_per_second",
    "decrease_rate_per_second",
    "extend_stream_end_time",
    "shorten_stream_end_time",
    # Withdrawal rate limiting (§3 callout 1)
    "create_stream_with_lookback",
    "set_lookback_window",
    "get_lookback_window",
    # Delegated withdrawal / cancellation (old, non-delegate_* variants, §3 callout 2)
    "delegated_withdraw",
    "delegated_cancel",
    "get_delegated_nonce",
    "get_delegated_cancel_nonce",
    "batch_withdraw_to",
    "withdraw_to",
    # Keeper cancellation (§3 callout 3)
    "keeper_cancel",
    "bulk_cancel_streams",
    "close_completed_stream",
    "close_cancelled_stream",
}

# --- Signature-shape checks (from MIGRATION.md §4) --------------------------
# (function_name, required_param, required_type)
SIGNATURE_CHECKS = [
    ("create_stream", "token", "Address"),
    ("create_stream", "cancellable", "bool"),
    ("create_stream", "pausable", "bool"),
    ("create_stream", "transferable", "bool"),
    ("withdraw", "amount", "Option<i128>"),
    ("top_up", "amount", "i128"),
]


# ---------------------------------------------------------------------------
# ABI loading
# ---------------------------------------------------------------------------

def load_abi(path: Path | None = None) -> dict:
    """Load and return the v1 ABI JSON as a dict."""
    abi_path = path or ABI_PATH
    if not abi_path.exists():
        raise FileNotFoundError(f"ABI file not found: {abi_path}")
    return json.loads(abi_path.read_text(encoding="utf-8"))


def abi_entrypoints(abi: dict) -> list[str]:
    """Return the sorted list of public function names from the ABI."""
    return sorted(f["name"] for f in abi.get("functions", []))


def abi_function(abi: dict, name: str) -> dict | None:
    """Return the function entry from the ABI with *name*, or None."""
    for f in abi.get("functions", []):
        if f["name"] == name:
            return f
    return None


def abi_input_types(abi: dict, name: str) -> list[str]:
    """Return the list of type strings for each input parameter of *name*."""
    fn = abi_function(abi, name)
    if fn is None:
        return []
    return [p["type"] for p in fn.get("inputs", [])]


def classify_entrypoints(abi: dict) -> tuple[set[str], set[str]]:
    """Split ABI entrypoints into (core, delegation) sets."""
    names = set(abi_entrypoints(abi))
    core = {n for n in names if not n.startswith(DELEGATION_PREFIXES)}
    delegation = {n for n in names if n.startswith(DELEGATION_PREFIXES)}
    return core, delegation


# ---------------------------------------------------------------------------
# MIGRATION.md parsing
# ---------------------------------------------------------------------------

def parse_migration_doc(path: Path | None = None) -> str:
    """Read and return the raw text of MIGRATION.md."""
    migration_path = path or MIGRATION_PATH
    if not migration_path.exists():
        raise FileNotFoundError(f"MIGRATION.md not found: {migration_path}")
    return migration_path.read_text(encoding="utf-8")


def parse_renames_table(doc: str) -> list[tuple[str, str]]:
    """Extract (old, new) pairs from the renames table in §4.

    The table has rows of the form::

        | `cancel_stream(sender, id)` | `cancel(id)` | renamed; ... |

    We strip the parameter lists from both columns to get bare function names.
    ``update_recipient / accept_recipient_update`` has two old names for one new
    name; we take the first one.
    """
    renames_section = re.search(
        r"## 4\. Renames.*?\| old \| v1 \| change \|", doc, re.DOTALL
    )
    if renames_section is None:
        raise ValueError("Could not find renames table in MIGRATION.md §4")

    table_text = doc[renames_section.end():]

    # Stop at the next ## heading.
    next_heading = table_text.find("\n## ")
    if next_heading != -1:
        table_text = table_text[:next_heading]

    pairs = []
    for row in table_text.splitlines():
        row = row.strip()
        if not row.startswith("|"):
            continue
        cells = [c.strip() for c in row.strip("|").split("|")]
        if len(cells) < 3:
            continue
        if cells[0].startswith("---"):
            continue
        old_name = _extract_fn_name(cells[0])
        new_name = _extract_fn_name(cells[1])
        if old_name and new_name:
            pairs.append((old_name, new_name))
    return pairs


def _extract_fn_name(cell: str) -> str | None:
    """Extract the primary function name from a table cell.

    Cells look like `` `cancel_stream(sender, id)` `` or
    `` `update_recipient` / `accept_recipient_update` ``.
    """
    text = cell.replace("`", "")
    if "(" in text:
        text = text.split("(")[0]
    text = text.split("/")[0]
    text = text.strip()
    return text if text else None


def parse_entrypoint_count_claim(doc: str) -> int | None:
    """Extract the v1 entrypoint count claim from MIGRATION.md.

    Looks for text like "v1 exposes **16**" and returns the integer.
    Returns None if no claim is found.
    """
    match = re.search(r"v1 exposes.*?\*\*(\d+)\*\*", doc, re.DOTALL)
    if match:
        return int(match.group(1))
    return None


def parse_old_count_claim(doc: str) -> int | None:
    """Extract the old contract-set entrypoint count from MIGRATION.md.

    Looks for text like "exposed **145 entrypoints**" and returns the integer.
    """
    match = re.search(r"exposed\s+\*\*(\d+)\s+entrypoints", doc, re.DOTALL)
    if match:
        return int(match.group(1))
    return None


# ---------------------------------------------------------------------------
# Validation checks
# ---------------------------------------------------------------------------

def check_renames(abi: dict, doc: str) -> list[str]:
    """Verify that all renamed entrypoints are present/absent as documented.

    The renames table from MIGRATION.md §4 is parsed from the document text.
    For each (old, new) pair:
      - If old != new (pure rename): old must NOT be in the ABI, new MUST be.
      - If old == new (signature change only): new must be in the ABI.
    """
    errors = []

    try:
        renames = parse_renames_table(doc)
    except ValueError as e:
        return [f"Failed to parse renames table: {e}"]

    if not renames:
        return ["Renames table in MIGRATION.md §4 is empty or could not be parsed"]

    abi_names = set(abi_entrypoints(abi))

    for old_name, new_name in renames:
        if old_name != new_name:
            if old_name in abi_names:
                errors.append(
                    f"Rename violation: old function `{old_name}` still "
                    f"exists in v1 ABI (MIGRATION.md says it was renamed to `{new_name}`)"
                )
            if new_name not in abi_names:
                errors.append(
                    f"Rename violation: new function `{new_name}` (from "
                    f"`{old_name}`) is NOT in v1 ABI"
                )
        else:
            if new_name not in abi_names:
                errors.append(
                    f"Signature-change entry: `{new_name}` is NOT in v1 ABI"
                )

    return errors


def check_removed(abi: dict) -> list[str]:
    """Verify that all curated removed entrypoints are absent from the v1 ABI."""
    errors = []
    abi_names = set(abi_entrypoints(abi))

    for name in sorted(REMOVED_ENTRYPOINTS):
        if name in abi_names:
            errors.append(
                f"Removal violation: `{name}` is listed as removed in "
                f"MIGRATION.md §3 but still exists in the v1 ABI"
            )

    return errors


def check_signatures(abi: dict) -> list[str]:
    """Verify signature-level claims from the renames table (§4)."""
    errors = []

    for fn_name, param_name, expected_type in SIGNATURE_CHECKS:
        fn = abi_function(abi, fn_name)
        if fn is None:
            errors.append(
                f"Signature check: function `{fn_name}` not found in ABI"
            )
            continue

        inputs = fn.get("inputs", [])
        found = False
        for inp in inputs:
            if inp["name"] == param_name:
                if inp["type"] != expected_type:
                    errors.append(
                        f"Signature check: `{fn_name}.{param_name}` type is "
                        f"`{inp['type']}` but MIGRATION.md claims `{expected_type}`"
                    )
                found = True
                break
        if not found:
            errors.append(
                f"Signature check: `{fn_name}` has no parameter named "
                f"`{param_name}` (MIGRATION.md says it should)"
            )

    return errors


def check_entrypoint_count(abi: dict, doc: str) -> list[str]:
    """Verify the entrypoint count claim from MIGRATION.md §3."""
    errors = []

    core, delegation = classify_entrypoints(abi)
    abi_names = core | delegation

    doc_count = parse_entrypoint_count_claim(doc)
    if doc_count is not None:
        if len(core) != doc_count:
            errors.append(
                f"Entrypoint count: MIGRATION.md claims v1 exposes "
                f"{doc_count} core entrypoints, but the ABI has "
                f"{len(core)} core + {len(delegation)} delegation = "
                f"{len(abi_names)} total"
            )
    else:
        errors.append(
            "MIGRATION.md §3 does not state a v1 entrypoint count claim"
        )

    return errors


def check_old_count_claim(doc: str) -> list[str]:
    """Verify the old-contract count claim from MIGRATION.md §3."""
    errors = []

    old_count = parse_old_count_claim(doc)
    if old_count is None:
        errors.append(
            "MIGRATION.md §3 does not state the old entrypoint count"
        )
        return errors

    # The old contract set is gone — we can't enumerate it. But the doc should
    # state the correct figure.  145 = 100 stream + 16 factory + 29 governance.
    expected_old_total = 100 + 16 + 29
    if old_count != expected_old_total:
        errors.append(
            f"MIGRATION.md claims the old contract set had {old_count} "
            f"entrypoints, but the documented breakdown (100 stream + 16 "
            f"factory + 29 governance) sums to {expected_old_total}"
        )

    return errors


# ---------------------------------------------------------------------------
# Entry points
# ---------------------------------------------------------------------------

def validate(abi_path: Path | None = None,
             migration_path: Path | None = None) -> list[str]:
    """Run all validation checks and return a list of error strings."""
    errors = []

    try:
        abi = load_abi(abi_path)
    except FileNotFoundError as e:
        return [str(e)]

    try:
        doc = parse_migration_doc(migration_path)
    except FileNotFoundError as e:
        return [str(e)]

    errors.extend(check_renames(abi, doc))
    errors.extend(check_removed(abi))
    errors.extend(check_signatures(abi))
    errors.extend(check_entrypoint_count(abi, doc))
    errors.extend(check_old_count_claim(doc))

    return errors


def main() -> int:
    errors = validate()

    if errors:
        print(f"FAIL: {len(errors)} migration validation error(s):")
        for e in errors:
            print(f"  - {e}")
        return 1

    print("OK: docs/MIGRATION.md is consistent with the v1 ABI.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
