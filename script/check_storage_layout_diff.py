#!/usr/bin/env python3
"""
script/check_storage_layout_diff.py
====================================

Diff-aware storage layout checker that detects breaking ``DataKey`` enum
changes across ``contracts/stream/src/lib.rs`` and
``contracts/factory/src/lib.rs`` between two git refs (e.g. PR head vs.
merge-base).

What it checks
--------------
For each ``#[contracttype] pub enum DataKey { ... }`` found in the tracked
source files, the script parses the variant list from both the **base** ref
(typically ``origin/main`` or ``HEAD~1``) and the **head** ref (the PR
branch commit).  It then reports any of the following as a **breaking
change** (exit code 1):

  | Condition                                     | Example (base → head)                       |
  |-----------------------------------------------|---------------------------------------------|
  | Variant reordered – name at position N changed| ``Stream`` (idx 2) → ``Token`` (idx 2)     |
  | Variant removed                               | ``Config`` present in base, absent in head  |
  | Variant has a changed field shape             | ``Stream(u64)`` → ``Stream(Address)``       |
  | Variant inserted at an existing position      | New variant at index 2, pushing old idx 2→3 |

A **non-breaking** (strictly additive) change is allowed:

  | Condition                       | Example                                   |
  |---------------------------------|-------------------------------------------|
  | New variant appended at the end | base has 37 variants; head has 38 with    |
  |                                 | same first 37, new variant at index 37   |

Exit codes
----------
  0  No breaking ``DataKey`` changes detected (additive-only changes are OK).
  1  At least one breaking ``DataKey`` change detected.
  2  File could not be read or no ``DataKey`` enum was found in one of the
     expected files.

Usage
-----
  python3 script/check_storage_layout_diff.py \\
      --base origin/main --head feature-branch

  Without arguments the script defaults to comparing ``HEAD`` against the
  working tree (useful for local verification before committing).

Files checked
-------------
  - contracts/stream/src/lib.rs  (DataKey enum – stream storage layout)
  - contracts/factory/src/lib.rs (DataKey enum – factory storage layout)

Security assumptions
--------------------
  - The script only parses ``pub enum DataKey`` declarations matching a
    specific regex.  Refactoring the enum's formatting (adding/removing
    attributes, changing indentation) should not affect parsing as long as
    the body still matches.
  - A variant's discriminant is derived from its *declaration order* (0-based
    index), NOT from explicit numeric assignments (Soroban's ``#[contracttype]``
    derives discriminants from declaration order, which is the source of the
    stability guarantee).
  - The field-shape check compares the *textual representation* of the variant's
    payload, e.g. ``Stream(u64)`` vs ``Stream(Address)``.  It does NOT resolve
    imports or type aliases, so ``Stream(TokenAmount)`` and ``Stream(i128)`` are
    treated as different shapes even if ``TokenAmount`` is a type alias for
    ``i128``.  This is a conservative (safe) false-positive trade-off: the CI
    will flag it, and a human reviewer must confirm the shape is semantically
    identical.
  - Variants that carry *no payload* (unit variants like ``Config``) store no
    field information.  Their shape is always ``""`` (empty string) and any
    change between unit and payload-bearing is caught as a shape-change.
  - The script does NOT check the ``Stream`` struct itself, only ``DataKey``
    enums.  Structural changes to ``Stream`` that break backward
    compatibility are checked by the ``storage_key_compat`` Rust tests.

CI integration
--------------
This script is wired into ``.github/workflows/ci.yml`` as a required job
named ``storage-layout-diff``.  The job runs on every PR, comparing the PR
head against the merge-base (``origin/main``).  If it exits 1, the CI
pipeline fails, signalling that the PR would corrupt storage on upgrade.

References
----------
  - ``docs/storage.md`` — Storage layout evolution policy and discriminant table.
  - ``docs/ABI_STABILITY.md`` — ABI stability contract including frozen
    discriminants.
  - ``contracts/stream/src/versioning.rs`` — ``FROZEN_DISCRIMINANTS_V9`` and
    compile-time variable count checks.
"""

import argparse
import re
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Dict, List, Optional, Tuple


# ---------------------------------------------------------------------------
# Constants
# ---------------------------------------------------------------------------

#: Source files that contain DataKey enums we check.
TRACKED_PATHS: List[str] = [
    "contracts/stream/src/lib.rs",
    "contracts/factory/src/lib.rs",
]

#: Regex to locate ``pub enum DataKey { ... }`` bodies.
#: Captures the body (everything between the opening ``{`` and the closing ``}``).
_DATAKEY_ENUM_RE = re.compile(
    r"pub\s+enum\s+DataKey\s*\{(?P<body>[^}]*)\}",
    re.MULTILINE,
)

#: Regex to match a single variant line such as:
#:   ``Config,``
#:   ``Stream(u64),``
#:
#: We capture:
#:   group(1) = variant name (e.g. ``Config``)
#:   group(3) = optional parenthesised payload (e.g. ``(u64, Address)``)
#:             or empty string for unit variants.
#:
#: NOTE: Lines starting with ``///``, ``//``, ``#``, or blank lines / ``}``
#: are filtered out by ``parse_datakey`` *before* this regex is applied per
#: line, so the regex does not need to handle attributes or doc comments.
_VARIANT_LINE_RE = re.compile(
    r"^\s*"
    r"(?P<name>[A-Z][A-Za-z0-9_]*)"
    r"(?P<payload>\([^)]*\))?"
    r"\s*,?\s*(?:///.*)?$",
    re.MULTILINE,
)

#: Secondary, simpler variant parser for edge cases that the primary regex
#: may miss (e.g. variants with nested generics).
_FALLBACK_VARIANT_RE = re.compile(
    r"^\s*([A-Z][A-Za-z0-9_]*)\s*(\([^)]*\))?\s*,", re.MULTILINE
)


# ---------------------------------------------------------------------------
# Data types
# ---------------------------------------------------------------------------


@dataclass(frozen=True)
class VariantDef:
    """Represents one parsed DataKey variant."""

    name: str
    """Variant name, e.g. ``Config``, ``Stream``."""

    payload: str
    """Parenthesised payload string, e.g. ``(u64)``, ``(Address)``, or ``""``."""

    index: int
    """0-based discriminant index derived from declaration order."""


DiffResult = List[str]
"""Human-readable diagnostics for each breaking change found."""


# ---------------------------------------------------------------------------
# Parsing
# ---------------------------------------------------------------------------


def parse_datakey(body: str) -> List[VariantDef]:
    """Parse the body of a ``DataKey`` enum and return sorted ``VariantDef`` instances.

    Parameters
    ----------
    body : str
        The text between the opening ``{`` and closing ``}`` of the enum
        declaration (including surrounding whitespace and attributes).

    Returns
    -------
    List[VariantDef]
        Parsed variants in declaration order.

    Raises
    ------
    ValueError
        If the body contains no parsable variants.
    """
    variants: List[VariantDef] = []

    for raw_line in body.splitlines():
        # Strip comments, doc comments, leading whitespace.
        line = raw_line.strip()
        if not line or line == "}" or line.startswith("//") or line.startswith("#"):
            continue
        if line.startswith("///"):
            continue

        # Try primary parse first.
        m = _VARIANT_LINE_RE.match(line)
        if m:
            name = m.group("name")
            payload = m.group("payload") or ""
            variants.append(
                VariantDef(name=name, payload=payload, index=len(variants))
            )
            continue

        # Fallback parse.
        m = _FALLBACK_VARIANT_RE.match(line)
        if m:
            name = m.group(1)
            payload = m.group(2) or ""
            variants.append(
                VariantDef(name=name, payload=payload, index=len(variants))
            )
            continue

    if not variants:
        raise ValueError("No DataKey variants could be parsed from enum body")

    return variants


def extract_datakey_variants(source_text: str) -> List[VariantDef]:
    """Extract all DataKey variants from the full source text of a Rust file.

    Parameters
    ----------
    source_text : str
        The complete content of a ``.rs`` file.

    Returns
    -------
    List[VariantDef]
        Parsed variants, or empty list if no ``pub enum DataKey`` was found.
    """
    m = _DATAKEY_ENUM_RE.search(source_text)
    if not m:
        return []
    body = m.group("body")
    return parse_datakey(body)


# ---------------------------------------------------------------------------
# Git helpers
# ---------------------------------------------------------------------------


def read_file_from_git(
    ref: Optional[str],
    git_path: str,
    local_path: Optional[str] = None,
) -> Optional[str]:
    """Read file content from a git ref or local working tree.

    Parameters
    ----------
    ref : str or None
        Git ref (commit hash, branch name, tag).  ``None`` means read from
        the local working tree.
    git_path : str
        File path **relative to the repo root** — this is what ``git show``
        expects after the ``:`` separator.  Pass ``None`` for *local_path*
        when only git access is needed.
    local_path : str or None
        Absolute (or CWD-relative) path for reading from the local working
        tree when *ref* is ``None``.  Falls back to *git_path* if not given.

    Returns
    -------
    str or None
        File content as string, or ``None`` if the file does not exist at
        that ref / on disk.
    """
    if ref is None:
        # Read from local working tree.
        p = Path(local_path or git_path)
        if not p.exists():
            return None
        return p.read_text(encoding="utf-8")
    try:
        # ``git show`` expects paths relative to the repository root
        # (inside the git archive), not absolute filesystem paths.
        result = subprocess.check_output(
            ["git", "show", f"{ref}:{git_path}"],
            stderr=subprocess.DEVNULL,
        )
        return result.decode("utf-8")
    except subprocess.CalledProcessError:
        return None


# ---------------------------------------------------------------------------
# Diff analysis
# ---------------------------------------------------------------------------


def diff_datakey_variants(
    label: str,
    base_variants: List[VariantDef],
    head_variants: List[VariantDef],
) -> DiffResult:
    """Compare two lists of ``DataKey`` variants and return breaking-change
    diagnostics.

    Parameters
    ----------
    label : str
        Human-readable label for the enum (e.g. the file path).
    base_variants : list
        Variants from the base ref (e.g. ``origin/main``).
    head_variants : list
        Variants from the head ref (e.g. the PR branch).

    Returns
    -------
    DiffResult
        List of human-readable diagnostic messages.  Empty list means no
        breaking changes detected.
    """
    results: DiffResult = []
    base_count = len(base_variants)
    head_count = len(head_variants)

    # Check for variant removal / reordering / field-shape change within the
    # overlapping range (min(base_count, head_count)).
    overlap = min(base_count, head_count)

    for idx in range(overlap):
        bv = base_variants[idx]
        hv = head_variants[idx]

        if bv.name != hv.name:
            results.append(
                f"[BREAKING] {label}: variant at index {idx} renamed — "
                f"'{bv.name}' (base) → '{hv.name}' (head).  "
                f"This shifts all subsequent discriminants."
            )
            continue

        if bv.payload != hv.payload:
            results.append(
                f"[BREAKING] {label}: variant '{bv.name}' at index {idx} "
                f"changed shape — {bv.payload or '(unit)'} (base) → "
                f"{hv.payload or '(unit)'} (head).  "
                f"Existing storage entries with this key will not decode."
            )

    # If the head has *fewer* variants than the base, at least one variant
    # was removed (the overlap check above will only catch a rename of the
    # last base variant; we need to explicitly flag the disappearance).
    if head_count < base_count:
        removed = [base_variants[i].name for i in range(head_count, base_count)]
        results.append(
            f"[BREAKING] {label}: {base_count - head_count} variant(s) "
            f"removed: {', '.join(removed)}.  A removed variant orphans "
            f"existing storage entries."
        )

    # If the head has *more* variants than the base, the new ones must be
    # strictly appended at the end.  The overlap check already verified that
    # the first ``base_count`` variants match, so we only need to confirm the
    # extra ones are truly new (which is fine by definition).
    if head_count > base_count:
        added = [head_variants[i].name for i in range(base_count, head_count)]
        results.append(
            f"[INFO] {label}: {head_count - base_count} new variant(s) "
            f"appended at the end: {', '.join(added)}.  "
            f"Additive-only change — OK."
        )

    return results


def check_file(
    label: str,
    base_text: Optional[str],
    head_text: Optional[str],
) -> DiffResult:
    """Run the storage layout diff check for a single source file.

    Parameters
    ----------
    label : str
        Human-readable label (usually the file path).
    base_text : str or None
        File content at the base ref (``None`` if file did not exist).
    head_text : str or None
        File content at the head ref (``None`` if file does not exist).

    Returns
    -------
    DiffResult
        Diagnostic messages.
    """
    results: DiffResult = []

    if base_text is None and head_text is None:
        results.append(
            f"[SKIP] {label}: file does not exist at either ref — skipping."
        )
        return results

    if head_text is None:
        results.append(
            f"[ERROR] {label}: file exists at base ref but was deleted in head. "
            f"This is a breaking storage-layout change."
        )
        return results

    base_variants = extract_datakey_variants(base_text) if base_text else []
    head_variants = extract_datakey_variants(head_text)

    if not head_variants:
        results.append(
            f"[ERROR] {label}: no DataKey enum found in head version. "
            f"If the enum was removed, this is a breaking change."
        )
        return results

    if not base_variants and base_text is not None:
        results.append(
            f"[ERROR] {label}: no DataKey enum found in base version but "
            f"file exists.  Cannot perform diff — assume non-breaking."
        )
        return results

    if not base_variants and base_text is None:
        # File was added in head — purely additive, no breaking change.
        results.append(
            f"[INFO] {label}: new file added at head with DataKey enum "
            f"({len(head_variants)} variants).  New contract deployment — "
            f"no existing storage to break."
        )
        return results

    results.extend(diff_datakey_variants(label, base_variants, head_variants))
    return results


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------


def build_arg_parser() -> argparse.ArgumentParser:
    """Build the argument parser."""
    p = argparse.ArgumentParser(
        description=__doc__.split("\n")[0],
        epilog=(
            "Exit codes: 0 = OK (additive-only changes allowed), "
            "1 = breaking change detected, 2 = configuration error."
        ),
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    p.add_argument(
        "--base",
        default="HEAD",
        help="Base git ref (default: HEAD, use 'origin/main' in CI)",
    )
    p.add_argument(
        "--head",
        default=None,
        help="Head git ref (default: working tree)",
    )
    p.add_argument(
        "--files",
        nargs="*",
        default=TRACKED_PATHS,
        help=f"Files to check (default: {' '.join(TRACKED_PATHS)})",
    )
    p.add_argument(
        "--repo-root",
        default=None,
        help=(
            "Path to repository root (auto-detected from script location if "
            "not provided)."
        ),
    )
    return p


def main(argv: Optional[List[str]] = None) -> int:
    """Entry point for the storage layout diff checker.

    Returns
    -------
    int
        Exit code (0 = OK, 1 = breaking change, 2 = configuration error).
    """
    parser = build_arg_parser()
    args = parser.parse_args(argv)

    # Determine repo root.
    if args.repo_root:
        repo_root = Path(args.repo_root).resolve()
    else:
        repo_root = Path(__file__).resolve().parent.parent

    files = args.files
    base_ref = args.base
    head_ref = args.head

    print(f"Storage layout diff check: {base_ref} ↔ {head_ref or '(working tree)'}")
    print(f"Repository root: {repo_root}")
    print(f"Files to check: {', '.join(files)}")
    print()

    all_results: DiffResult = []
    any_breaking = False

    for rel_path in files:
        full_path = repo_root / rel_path
        label = str(rel_path)

        # git show expects repo-relative paths; local file reads need the
        # absolute path.  We pass the relative path for git operations
        # and fall back to absolute for local reads.
        git_rel_path = str(rel_path)
        local_abs_path = str(full_path)

        base_text = read_file_from_git(base_ref, git_rel_path, local_abs_path)
        head_text = read_file_from_git(head_ref, git_rel_path, local_abs_path)

        results = check_file(label, base_text, head_text)
        all_results.extend(results)

        for msg in results:
            if msg.startswith("[BREAKING]") or msg.startswith("[ERROR]"):
                any_breaking = True
            print(msg)

    print()
    if any_breaking:
        print(
            "FAILURE: Breaking DataKey storage-layout change detected.\n"
            "This PR would corrupt existing persistent storage on upgrade.\n"
            "See docs/storage.md for the DataKey evolution policy."
        )
        return 1

    print("PASS: No breaking DataKey storage-layout changes detected.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
