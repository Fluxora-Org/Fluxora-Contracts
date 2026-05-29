#!/usr/bin/env python3
"""
validate-doc-alignment.py

Validates that integration-critical identifiers defined in Rust contract source
files are documented in the corresponding Markdown documentation files.

Checks three categories:
  1. Public entrypoints  (pub fn <name>) in contracts/stream/src/lib.rs
     -> must appear in docs/streaming.md
  2. Event symbols       (Symbol::short/new) in contracts/core/src/events.rs
     -> must appear in docs/events.md
  3. Error enum variants in contracts/core/src/error.rs
     -> must appear in docs/error.md
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
}

# pub fn names that are internal helpers or common traits, not ABI entry-points.
ENTRYPOINT_ALLOWLIST = frozenset({
    "save_stream",
    "require_not_paused",
    "require_not_globally_paused",
})

# `#[contracterror]`-shaped variants that belong to other enums in the same file.
ERROR_EXTRACT_EXCLUDE = frozenset(
    {"Operational", "Administrative", "Compliance", "Emergency"}
)

# ---------------------------------------------------------------------------
# Path resolution
# ---------------------------------------------------------------------------

def resolve_path(name: str, canonical: Path, glob_pattern: str) -> Path | None:
    """Return a resolved Path for a required file."""
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

# ---------------------------------------------------------------------------
# Validation
# ---------------------------------------------------------------------------

def check_missing(identifiers: set, doc_text: str) -> set:
    return {ident for ident in identifiers if ident not in doc_text}

def validate(
    contract_path: Path,
    events_path: Path,
    error_path: Path,
    streaming_doc: Path,
    events_doc: Path,
    error_doc: Path,
) -> int:
    """Run all alignment checks. Returns 0 on success, 1 on any drift."""
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
    )

if __name__ == "__main__":
    sys.exit(main())