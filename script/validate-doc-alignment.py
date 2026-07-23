#!/usr/bin/env python3
"""
validate-doc-alignment.py

Validates that integration-critical identifiers defined in Rust contract source
files are documented in the corresponding Markdown documentation files.

Checks four categories:
  1. Public entrypoints  (pub fn <name>) in contracts/stream/src/lib.rs
     -> must appear in docs/streaming.md
  2. Event symbols       (Symbol::short/new) in contracts/stream/src/lib.rs
     -> must appear in docs/events.md
  3. Error enum variants in contracts/stream/src/lib.rs
     -> must appear in docs/error.md
  4. ABI entrypoints     (pub fn inside #[contractimpl] block) in lib.rs
     -> must appear in docs/audit.md entrypoint table

Security assumptions
--------------------
- All input files are local, trusted repository sources.  This script is
  intended for CI use only; do not pipe untrusted Rust source into it.
- Regex patterns are applied to file contents already loaded into memory;
  there is no shell execution of file content.
- ``pathlib.Path.read_text`` is used with explicit ``encoding='utf-8'`` to
  prevent locale-dependent decoding surprises.
- ``REPO_ROOT`` is derived from ``__file__``, not from environment variables,
  to prevent path-injection via CI environment configuration.

Usage
-----
    python3 script/validate-doc-alignment.py

Exit codes
----------
    0 — all checks passed; no documentation drift detected.
    1 — one or more identifiers are missing from their documentation target,
        or a required source/doc file could not be located.
"""

from __future__ import annotations

import os
import re
import sys
from pathlib import Path

# ---------------------------------------------------------------------------
# Project root — derived from this file's location.
# ---------------------------------------------------------------------------

REPO_ROOT = Path(__file__).resolve().parent.parent

# ---------------------------------------------------------------------------
# MAPPING: logical name -> (canonical relative path, glob fallback pattern)
# Updated to match the "contracts/stream" directory seen in your logs.
# ---------------------------------------------------------------------------

MAPPING = {
    "CONTRACT_SRC": (
        REPO_ROOT / "contracts" / "stream" / "src" / "lib.rs",
        "**/stream/src/lib.rs",
    ),
    "EVENTS_SRC": (
        REPO_ROOT / "contracts" / "stream" / "src" / "lib.rs",
        "**/stream/src/lib.rs",
    ),
    "ERROR_SRC": (
        REPO_ROOT / "contracts" / "stream" / "src" / "lib.rs",
        "**/stream/src/lib.rs",
    ),
    "DOC_STREAMING": (
        REPO_ROOT / "docs" / "streaming.md",
        "**/docs/streaming.md",
    ),
    "DOC_EVENTS": (
        REPO_ROOT / "docs" / "events.md",
        "**/docs/events.md",
    ),
    "DOC_ERROR": (
        REPO_ROOT / "docs" / "error.md",
        "**/docs/error.md",
    ),
    "DOC_AUDIT": (
        REPO_ROOT / "docs" / "audit.md",
        "**/docs/audit.md",
    ),
}

# pub fn names that are internal helpers or common traits, not ABI entry-points.
# These are suppressed from the streaming.md entrypoint check.
ENTRYPOINT_ALLOWLIST = frozenset({
    "save_stream",
    "require_not_paused",
    "require_not_globally_paused",
})

# pub fn names inside #[contractimpl] that are intentionally excluded from the
# audit.md table.  Add names here only when they are provably internal shims
# with no external ABI surface (e.g. upgrade helpers gated behind a separate
# auth path that the auditor needs not enumerate per-entrypoint).
#
# Each exclusion must have a justification comment.
AUDIT_ENTRYPOINT_ALLOWLIST = frozenset({
    # Soroban upgrade helper — not a user-facing ABI entry; gated by
    # admin auth and invoked through the host's upgrade mechanism.
    # Documented separately in docs/upgrade.md.
    "upgrade",
    # Pure arithmetic helper exposed as pub for test-crate access.
    # Contains no state reads/writes and emits no events.
    "compute_keeper_fee_split",
})

# `#[contracterror]`-shaped variants that belong to other enums in the same file.
ERROR_EXTRACT_EXCLUDE = frozenset(
    {"Operational", "Administrative", "Compliance", "Emergency", "GlobalEmergency"}
)

# ---------------------------------------------------------------------------
# Path resolution
# ---------------------------------------------------------------------------

def resolve_path(name: str, canonical: Path, glob_pattern: str) -> Path | None:
    """Return a resolved Path for a required file.

    First tries the canonical absolute path; if that file does not exist,
    falls back to a recursive glob from ``REPO_ROOT``.  Returns ``None`` if
    neither strategy finds a file.

    Args:
        name:         Logical name used for diagnostic messages.
        canonical:    The expected absolute path of the file.
        glob_pattern: A ``**``-style glob pattern rooted at ``REPO_ROOT``.

    Returns:
        A ``Path`` pointing to the located file, or ``None``.
    """
    if canonical.exists():
        return canonical

    # If canonical fails, search recursively from REPO_ROOT
    matches = sorted(REPO_ROOT.glob(glob_pattern))
    if matches:
        return matches[0]

    return None

def _print_debug_tree(root: Path, max_depth: int = 4) -> None:
    """Print a lightweight directory tree to stdout for CI debugging."""
    print(f"   [CWD] {os.getcwd()}")
    print(f"   [ROOT] {root}")
    for item in sorted(root.rglob("*")):
        try:
            rel = item.relative_to(root)
        except ValueError:
            continue
        depth = len(rel.parts)
        if depth > max_depth:
            continue
        indent = "  " + ("  " * (depth - 1))
        marker = "/" if item.is_dir() else ""
        print(f"{indent}{rel.name}{marker}")

def resolve_all() -> tuple[dict, bool]:
    """Resolve every entry in MAPPING and diagnostic logging on failure."""
    resolved = {}
    missing_any = False

    for name, (canonical, glob_pattern) in MAPPING.items():
        path = resolve_path(name, canonical, glob_pattern)
        if path is None:
            print(f"[FILE MISSING]: Could not locate {name}. Tried canonical: {canonical} and glob: {glob_pattern}")
            missing_any = True
        else:
            resolved[name] = path

    if missing_any:
        print("\n--- Repository structure (debug) ---")
        _print_debug_tree(REPO_ROOT)
        print("------------------------------------\n")

    return resolved, not missing_any

# ---------------------------------------------------------------------------
# Extraction helpers
# ---------------------------------------------------------------------------

_RE_ENTRYPOINT = re.compile(
    r"^\s*pub\s+fn\s+([a-zA-Z0-9_]+)\s*[\(<]",
    re.MULTILINE,
)

_RE_EVENT_SYMBOL = re.compile(
    r'(?:Symbol::(?:short|new)\s*\(\s*&\w+\s*,\s*"([^"]+)"\s*\)'
    r'|symbol_short!\(\s*"([^"]+)"\s*\))',
    re.MULTILINE,
)

_RE_ERROR_VARIANT = re.compile(
    r"^\s{4}([A-Z][A-Za-z0-9]+)\s*=\s*\d+\s*,",
    re.MULTILINE,
)

_RE_ERROR_DISCRIMINANT = re.compile(
    r"^\s{4}([A-Z][A-Za-z0-9]+)\s*=\s*(\d+)\s*,",
    re.MULTILINE,
)

# Matches the body of `pub enum ContractError { ... }` (non-greedy on braces).
_RE_CONTRACT_ERROR_BODY = re.compile(
    r"pub\s+enum\s+ContractError\s*\{([^}]*)\}",
    re.DOTALL,
)

# Regex to extract the text content of the #[contractimpl] block.
#
# Strategy: locate the `#[contractimpl]` attribute, then consume the
# following `impl <Ident> {` header, then capture everything up to the
# matching closing brace at depth 0.  The body capture is intentionally
# not done with a single regex (brace-matching requires a state machine);
# instead, ``extract_contractimpl_entrypoints`` drives a character-level
# scan after a simple header match.
_RE_CONTRACTIMPL_HEADER = re.compile(
    r"#\[contractimpl\]\s*impl\s+\w+\s*\{",
    re.DOTALL,
)

def extract_contractimpl_entrypoints(source: str) -> set[str]:
    """Parse pub fn names from the ``#[contractimpl]`` block only.

    Walks the source character-by-character after finding the
    ``#[contractimpl] impl <Name> {`` header to perform brace-balanced
    extraction.  This avoids false positives from private helper
    ``impl`` blocks or module-level free functions that appear elsewhere
    in the same file.

    The result is filtered through ``AUDIT_ENTRYPOINT_ALLOWLIST`` to
    exclude entries that are intentionally omitted from ``docs/audit.md``.

    Args:
        source: Full text content of ``lib.rs``.

    Returns:
        Set of pub fn names that form the on-chain ABI surface and must
        be documented in ``docs/audit.md``.
    """
    m = _RE_CONTRACTIMPL_HEADER.search(source)
    if not m:
        return set()

    # Start scanning from the opening brace of the impl block.
    start = m.end() - 1  # points at the '{' of `impl Foo {`
    depth = 0
    block_start = start
    block_end = len(source)

    for i in range(start, len(source)):
        ch = source[i]
        if ch == "{":
            depth += 1
        elif ch == "}":
            depth -= 1
            if depth == 0:
                block_end = i
                break

    block = source[block_start:block_end]
    names = set(_RE_ENTRYPOINT.findall(block))
    return names - AUDIT_ENTRYPOINT_ALLOWLIST


def extract_audit_entrypoints_from_doc(doc_text: str) -> set[str]:
    """Parse entrypoint names listed in the ``docs/audit.md`` table.

    Looks for Markdown table rows where the first column contains a
    backtick-quoted identifier, e.g.:

        | `init` | ... |

    This mirrors how ``docs/audit.md`` is currently structured and how
    auditors are expected to maintain it.

    Args:
        doc_text: Full text content of ``docs/audit.md``.

    Returns:
        Set of entrypoint name strings found in the table.
    """
    # Match the first cell of a Markdown table row: | `identifier` |
    pattern = re.compile(r"^\s*\|\s*`([a-zA-Z0-9_]+)`", re.MULTILINE)
    return set(pattern.findall(doc_text))


def extract_entrypoints(source: str) -> set:
    names = set(_RE_ENTRYPOINT.findall(source))
    return names - ENTRYPOINT_ALLOWLIST

def extract_event_symbols(source: str) -> set:
    out: set[str] = set()
    for a, b in _RE_EVENT_SYMBOL.findall(source):
        if a:
            out.add(a)
        if b:
            out.add(b)
    return out

def extract_error_variants(source: str) -> set:
    return set(_RE_ERROR_VARIANT.findall(source)) - ERROR_EXTRACT_EXCLUDE

def check_duplicate_discriminants(source: str) -> bool:
    """Parse only the ContractError enum body and fail if any two variants share a discriminant."""
    m = _RE_CONTRACT_ERROR_BODY.search(source)
    if not m:
        print("WARNING: could not locate 'pub enum ContractError' in error source")
        return False
    body = m.group(1)
    matches = _RE_ERROR_DISCRIMINANT.findall(body)
    seen = {}
    duplicate_found = False
    for variant, val in matches:
        if variant in ERROR_EXTRACT_EXCLUDE:
            continue
        if val in seen:
            print(f"DUPLICATE DISCRIMINANT: '{variant}' and '{seen[val]}' both use value {val}")
            duplicate_found = True
        else:
            seen[val] = variant
    return duplicate_found

# ---------------------------------------------------------------------------
# Validation
# ---------------------------------------------------------------------------

def check_missing(identifiers: set, doc_text: str) -> set:
    return {ident for ident in identifiers if ident not in doc_text}


def check_audit_md_entrypoint_drift(
    source: str,
    audit_doc_text: str,
    audit_doc_path: Path,
) -> bool:
    """Check that every ABI entrypoint in the ``#[contractimpl]`` block is
    listed in ``docs/audit.md``'s entrypoint table.

    Emits one ``MISSING AUDIT DOC:`` line per absent entrypoint and returns
    ``True`` if any drift is found.

    The check is **additive-only**: it verifies that the doc is a *superset*
    of the code surface, not an exact match.  Extra rows in audit.md that no
    longer exist in code are not flagged here (a separate "stale doc" check
    could complement this).

    Args:
        source:          Full text of ``contracts/stream/src/lib.rs``.
        audit_doc_text:  Full text of ``docs/audit.md``.
        audit_doc_path:  Path to ``docs/audit.md`` (used in messages only).

    Returns:
        ``True`` if one or more entrypoints are missing from the table,
        ``False`` if the table is complete.
    """
    code_entrypoints = extract_contractimpl_entrypoints(source)
    doc_entrypoints = extract_audit_entrypoints_from_doc(audit_doc_text)

    missing = {name for name in code_entrypoints if name not in doc_entrypoints}
    if not missing:
        return False

    try:
        display = audit_doc_path.relative_to(REPO_ROOT)
    except ValueError:
        display = audit_doc_path

    for name in sorted(missing):
        print(
            f"MISSING AUDIT DOC: '{name}' (ABI entrypoint) is implemented in "
            f"lib.rs but absent from the entrypoint table in '{display}'"
        )
    return True


def validate(
    contract_path: Path,
    events_path: Path,
    error_path: Path,
    streaming_doc: Path,
    events_doc: Path,
    error_doc: Path,
    audit_doc: Path | None = None,
) -> int:
    """Run all alignment checks. Returns 0 on success, 1 on any drift.

    Args:
        contract_path:  Path to ``contracts/stream/src/lib.rs``.
        events_path:    Path to the events source file (currently also lib.rs).
        error_path:     Path to the error source file (currently also lib.rs).
        streaming_doc:  Path to ``docs/streaming.md``.
        events_doc:     Path to ``docs/events.md``.
        error_doc:      Path to ``docs/error.md``.
        audit_doc:      Path to ``docs/audit.md`` (optional; skipped if None).

    Returns:
        ``0`` if every check passes, ``1`` if any drift is detected.
    """
    source = contract_path.read_text(encoding="utf-8")
    events_source = events_path.read_text(encoding="utf-8")
    error_source = error_path.read_text(encoding="utf-8")
    streaming_text = streaming_doc.read_text(encoding="utf-8")
    events_text = events_doc.read_text(encoding="utf-8")
    error_text = error_doc.read_text(encoding="utf-8")

    checks = [
        (extract_entrypoints(source), streaming_text, streaming_doc, "entrypoint"),
        (extract_event_symbols(events_source), events_text, events_doc, "event symbol"),
        (extract_error_variants(error_source), error_text, error_doc, "error variant"),
    ]

    drift_found = False

    for identifiers, doc_text, doc_path, kind in checks:
        for ident in sorted(check_missing(identifiers, doc_text)):
            try:
                display = doc_path.relative_to(REPO_ROOT)
            except ValueError:
                display = doc_path
            print(f"MISSING DOC: '{ident}' ({kind}) found in code but not in '{display}'")
            drift_found = True

    if check_duplicate_discriminants(error_source):
        drift_found = True

    # ------------------------------------------------------------------
    # audit.md entrypoint drift check
    # ------------------------------------------------------------------
    if audit_doc is not None:
        audit_text = audit_doc.read_text(encoding="utf-8")
        if check_audit_md_entrypoint_drift(source, audit_text, audit_doc):
            drift_found = True

    if not drift_found:
        print("OK: all contract identifiers are present in documentation.")

    return 1 if drift_found else 0

def main() -> int:
    resolved, ok = resolve_all()
    if not ok:
        return 1

    return validate(
        resolved["CONTRACT_SRC"],
        resolved["EVENTS_SRC"],
        resolved["ERROR_SRC"],
        resolved["DOC_STREAMING"],
        resolved["DOC_EVENTS"],
        resolved["DOC_ERROR"],
        audit_doc=resolved.get("DOC_AUDIT"),
    )

if __name__ == "__main__":
    sys.exit(main())
