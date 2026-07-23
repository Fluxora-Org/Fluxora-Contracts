#!/usr/bin/env python3
"""
check_snapshot_diff.py

Classify whether a snapshot diff between two contract snapshot JSON files
contains security-relevant field changes that require extra reviewer scrutiny.

Security-relevant fields are those that govern:
  - Authorization paths  (admin address, token contract address)
  - Rate and accrual limits  (rate_per_second, max_rate_per_second, deposit amounts)
  - Token identity and trust  (token address / token_address key)
  - Pause and emergency state  (any pause or emergency flag)
  - Storage key discriminants  (DataKey layout changes)
  - Recipient rotation and delegation  (recipient, pending_recipient_update, nonce)

Exit-code contract
------------------
  0  No security-relevant field changes detected.
  1  One or more security-relevant field changes detected; mandatory extra review.
  2  Usage error (bad arguments, missing files, invalid JSON).

Usage
-----
  python script/check_snapshot_diff.py --base <base.json> --head <head.json>
  python script/check_snapshot_diff.py --base <base.json> --head <head.json> --quiet
  python script/check_snapshot_diff.py --base <base.json> --head <head.json> --output-format json

Notes
-----
  - This tool is NOT wired into CI yet. The companion CI-wiring issue must land
    before these checks are enforced automatically. Until then, maintainers must
    run this script manually during PR review.
  - Comparison is recursive: nested JSON objects and arrays are flattened into
    dotted key paths (e.g. "config.admin", "streams.0.rate_per_second") before
    matching against SECURITY_FIELDS.
  - Matching is case-insensitive substring/exact match; see is_security_relevant()
    for the full algorithm.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Any, Iterator

# ---------------------------------------------------------------------------
# Security field registry
# ---------------------------------------------------------------------------

# Each entry is a lowercase string.  A field key is security-relevant when
# it *equals* one of these values OR *contains* one of them as a substring.
# Keep this list sorted for readability and deterministic iteration.
SECURITY_FIELDS: frozenset[str] = frozenset(
    {
        # Authorization / admin
        "admin",
        "admin_address",
        "require_auth",
        # Token identity and trust
        "token",
        "token_address",
        "token_contract",
        # Rate / accrual caps
        "rate_per_second",
        "max_rate_per_second",
        "deposit_amount",
        # Recipient and delegation
        "recipient",
        "pending_recipient_update",
        "delegated_nonce",
        "nonce",
        # Pause / emergency state
        "paused",
        "emergency",
        "pause_state",
        "creation_paused",
        "global_emergency_paused",
        # Storage layout sentinels
        "data_key",
        "contract_version",
    }
)


# ---------------------------------------------------------------------------
# Field classification
# ---------------------------------------------------------------------------


def is_security_relevant(field_key: str) -> bool:
    """Return True if *field_key* matches any entry in SECURITY_FIELDS.

    Matching algorithm (applied to the *lowercased* field key):

    1. Exact match  — the key equals a SECURITY_FIELDS entry verbatim.
    2. Substring match — the key contains a SECURITY_FIELDS entry as a
       substring.  This catches composite keys such as "stream.admin" or
       "config.max_rate_per_second" without requiring every variant to be
       enumerated explicitly.

    The dotted-path prefix from recursive flattening (e.g. "streams.0.") is
    intentionally included in the key so that the substring match works
    transparently for nested structures.

    Example
    -------
    >>> is_security_relevant("config.admin")
    True
    >>> is_security_relevant("config.start_time")
    False
    >>> is_security_relevant("status")
    False
    >>> is_security_relevant("global_emergency_paused")
    True
    """
    lowered = field_key.lower()
    for sentinel in SECURITY_FIELDS:
        # Exact match
        if lowered == sentinel:
            return True
        # Substring match — the sentinel appears anywhere in the dotted path
        if sentinel in lowered:
            return True
    return False


# ---------------------------------------------------------------------------
# JSON flattening
# ---------------------------------------------------------------------------


def _flatten(obj: Any, prefix: str = "") -> Iterator[tuple[str, Any]]:
    """Recursively flatten a JSON value into (dotted_key, leaf_value) pairs.

    dict  → recurse with updated prefix
    list  → recurse with index-based suffix
    other → yield (prefix, value)

    Empty dicts and lists yield nothing.
    """
    if isinstance(obj, dict):
        for k, v in obj.items():
            new_key = f"{prefix}.{k}" if prefix else k
            yield from _flatten(v, new_key)
    elif isinstance(obj, list):
        for i, v in enumerate(obj):
            new_key = f"{prefix}.{i}" if prefix else str(i)
            yield from _flatten(v, new_key)
    else:
        yield prefix, obj


def flatten_snapshot(snapshot: dict[str, Any]) -> dict[str, Any]:
    """Return a flat {dotted_key: value} dict from a snapshot object."""
    return dict(_flatten(snapshot))


# ---------------------------------------------------------------------------
# Diff computation
# ---------------------------------------------------------------------------


def compute_diff(
    base: dict[str, Any],
    head: dict[str, Any],
) -> dict[str, dict[str, Any]]:
    """Compute the semantic diff between two flat snapshot dicts.

    Returns a dict with three keys:
      "added"   — keys present in head but not in base
      "removed" — keys present in base but not in head
      "changed" — keys present in both but with different values

    Each value is itself a dict mapping field_key → {"base": ..., "head": ...}
    for "changed", or field_key → value for "added"/"removed".
    """
    base_flat = flatten_snapshot(base)
    head_flat = flatten_snapshot(head)

    base_keys = set(base_flat)
    head_keys = set(head_flat)

    added: dict[str, Any] = {k: head_flat[k] for k in head_keys - base_keys}
    removed: dict[str, Any] = {k: base_flat[k] for k in base_keys - head_keys}
    changed: dict[str, dict[str, Any]] = {
        k: {"base": base_flat[k], "head": head_flat[k]}
        for k in base_keys & head_keys
        if base_flat[k] != head_flat[k]
    }

    return {"added": added, "removed": removed, "changed": changed}


# ---------------------------------------------------------------------------
# Security classification of a diff
# ---------------------------------------------------------------------------


def find_security_relevant_changes(
    diff: dict[str, dict[str, Any]],
) -> list[dict[str, Any]]:
    """Return a list of security-relevant change records from *diff*.

    Each record is:
      {
        "key":       <dotted field path>,
        "change_type": "added" | "removed" | "changed",
        "base":      <old value or None>,
        "head":      <new value or None>,
      }

    Records are sorted by key for deterministic output.
    """
    hits: list[dict[str, Any]] = []

    for k, v in diff["added"].items():
        if is_security_relevant(k):
            hits.append(
                {"key": k, "change_type": "added", "base": None, "head": v}
            )

    for k, v in diff["removed"].items():
        if is_security_relevant(k):
            hits.append(
                {"key": k, "change_type": "removed", "base": v, "head": None}
            )

    for k, v in diff["changed"].items():
        if is_security_relevant(k):
            hits.append(
                {
                    "key": k,
                    "change_type": "changed",
                    "base": v["base"],
                    "head": v["head"],
                }
            )

    return sorted(hits, key=lambda r: r["key"])


# ---------------------------------------------------------------------------
# I/O helpers
# ---------------------------------------------------------------------------


def load_snapshot(path: Path) -> dict[str, Any]:
    """Load and parse a JSON snapshot file.  Raises SystemExit(2) on error."""
    try:
        text = path.read_text(encoding="utf-8")
    except OSError as exc:
        _exit_usage(f"Cannot read file '{path}': {exc}")

    try:
        obj = json.loads(text)
    except json.JSONDecodeError as exc:
        _exit_usage(f"Invalid JSON in '{path}': {exc}")

    if not isinstance(obj, dict):
        _exit_usage(
            f"Expected a JSON object at the top level of '{path}', "
            f"got {type(obj).__name__}"
        )
    return obj  # type: ignore[return-value]


def _exit_usage(message: str) -> None:
    """Print *message* to stderr and exit with code 2 (usage error)."""
    print(f"error: {message}", file=sys.stderr)
    sys.exit(2)


# ---------------------------------------------------------------------------
# Output formatting
# ---------------------------------------------------------------------------


def format_human(
    hits: list[dict[str, Any]],
    base_path: str,
    head_path: str,
) -> str:
    """Format security-relevant changes as human-readable text."""
    lines: list[str] = []

    if not hits:
        lines.append("check_snapshot_diff: no security-relevant changes detected.")
        return "\n".join(lines)

    lines.append(
        f"check_snapshot_diff: {len(hits)} security-relevant change(s) detected."
    )
    lines.append(f"  base: {base_path}")
    lines.append(f"  head: {head_path}")
    lines.append("")
    lines.append("  Mandatory extra review required (see docs/snapshot-security-diff.md).")
    lines.append("")

    for rec in hits:
        tag = rec["change_type"].upper()
        key = rec["key"]
        if rec["change_type"] == "added":
            lines.append(f"  [{tag}]   {key} = {rec['head']!r}")
        elif rec["change_type"] == "removed":
            lines.append(f"  [{tag}] {key} = {rec['base']!r}")
        else:
            lines.append(
                f"  [{tag}] {key}  {rec['base']!r} → {rec['head']!r}"
            )

    return "\n".join(lines)


def format_json_output(
    hits: list[dict[str, Any]],
    base_path: str,
    head_path: str,
    flagged: bool,
) -> str:
    """Format the result as a JSON object suitable for machine consumption."""
    return json.dumps(
        {
            "flagged": flagged,
            "base": base_path,
            "head": head_path,
            "security_relevant_changes": hits,
        },
        indent=2,
        default=str,
    )


# ---------------------------------------------------------------------------
# CLI entry point
# ---------------------------------------------------------------------------


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        prog="check_snapshot_diff",
        description=(
            "Detect security-relevant field changes between two contract "
            "snapshot JSON files.  Exits 0 if no flagged changes, 1 if "
            "flagged, 2 on usage error."
        ),
    )
    parser.add_argument(
        "--base",
        required=True,
        metavar="FILE",
        help="Path to the base (before) snapshot JSON file.",
    )
    parser.add_argument(
        "--head",
        required=True,
        metavar="FILE",
        help="Path to the head (after) snapshot JSON file.",
    )
    parser.add_argument(
        "--quiet",
        action="store_true",
        default=False,
        help=(
            "Suppress all stdout output; only the exit code is meaningful.  "
            "Errors are still written to stderr."
        ),
    )
    parser.add_argument(
        "--output-format",
        choices=["human", "json"],
        default="human",
        metavar="FORMAT",
        help="Output format: 'human' (default) or 'json'.",
    )
    return parser


def main(argv: list[str] | None = None) -> int:
    """Entry point; returns the integer exit code."""
    parser = build_parser()
    args = parser.parse_args(argv)

    base_path = Path(args.base)
    head_path = Path(args.head)

    base_snapshot = load_snapshot(base_path)
    head_snapshot = load_snapshot(head_path)

    diff = compute_diff(base_snapshot, head_snapshot)
    hits = find_security_relevant_changes(diff)

    flagged = bool(hits)
    exit_code = 1 if flagged else 0

    if not args.quiet:
        if args.output_format == "json":
            output = format_json_output(hits, args.base, args.head, flagged)
        else:
            output = format_human(hits, args.base, args.head)
        print(output)

    return exit_code


if __name__ == "__main__":
    sys.exit(main())
