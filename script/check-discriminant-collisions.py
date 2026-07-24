#!/usr/bin/env python3
"""
script/check-discriminant-collisions.py
========================================

Cross-file discriminant-collision audit for docs/error.md.

Parses the discriminant tables from all three contract sections of
docs/error.md (ContractError / stream, FactoryError / factory, and
GovernanceError / governance) and reports:

  1. Intra-section collisions — two *different* variant names assigned the
     same numeric code within one enum (a docs bug, not necessarily a code
     bug, because Soroban/Rust forbids duplicate discriminants at the source
     level).

  2. Cross-section collisions — the same numeric code appears in more than
     one section.  This is only a real runtime problem when a shared off-chain
     decoder routes both error namespaces through a single code-to-message
     lookup; the script documents the finding either way.

  3. Out-of-order entries — discriminant values are not monotonically
     increasing within a section (can indicate a cut-paste mistake).

Usage
-----
    python3 script/check-discriminant-collisions.py [--docs PATH]

    --docs PATH   Path to docs/error.md (default: docs/error.md relative to
                  the repo root, auto-detected from this script's location).

Exit codes
----------
    0  No intra-section collisions found (cross-section overlaps are printed
       as warnings but do NOT cause a non-zero exit, because overlap across
       independent enums is permitted when no shared decoder exists).
    1  At least one intra-section collision (same code, different name, same
       enum) was found — this is always a documentation error.
    2  The docs file could not be read or no discriminant tables were found.
"""

from __future__ import annotations

import argparse
import re
import sys
from collections import defaultdict
from pathlib import Path
from typing import Dict, List, NamedTuple, Tuple


# ---------------------------------------------------------------------------
# Data types
# ---------------------------------------------------------------------------

class Entry(NamedTuple):
    """One row from a discriminant table."""
    code: int
    name: str
    line_no: int  # 1-based line number in the source file (for diagnostics)


Section = Dict[str, List[Entry]]  # section_label → list of entries


# ---------------------------------------------------------------------------
# Parsing
# ---------------------------------------------------------------------------

# Matches a Markdown table row of the form:
#
#   | 1 | `AlreadyInitialized` | ...
#   | `StreamNotFound` | 1 | ...
#
# The two supported column orderings are:
#   (a) numeric discriminant first, then backtick-quoted variant name
#   (b) backtick-quoted variant name first, then numeric discriminant
#
# Rows that match neither pattern are silently skipped.

_ROW_DISC_FIRST   = re.compile(r"^\|\s*(\d+)\s*\|\s*`([A-Za-z_][A-Za-z0-9_]*)`")
_ROW_NAME_FIRST   = re.compile(r"^\|\s*`([A-Za-z_][A-Za-z0-9_]*)`\s*\|\s*(\d+)\s*\|")

# Markdown table separator row (the |---|---| divider line).
_TABLE_SEP = re.compile(r"^\|[-|: ]+\|")

# Column-header rows that identify a *discriminant* table vs. a plain enum table.
# We only parse rows from tables whose header contains one of these keywords.
_DISC_HEADER_KEYWORDS = re.compile(
    r"error\s*code|discriminant|code\s*\|.*variant|variant\s*\|.*code",
    re.I,
)

# Section headers we care about.  The key is a human-readable label used in
# output; the value is a regex that matches the Markdown heading line.
_SECTION_PATTERNS: List[Tuple[str, re.Pattern]] = [
    ("ContractError (stream)",  re.compile(r"^#{1,4}\s+Error Code Reference Table", re.I)),
    ("FactoryError (factory)",  re.compile(r"^#{1,4}\s+.*FactoryError.*Reference.*Factory", re.I)),
    ("GovernanceError (governance)", re.compile(r"^#{1,4}\s+.*GovernanceError.*Reference", re.I)),
]

# A heading that starts a new top-level section (resets the current section).
_ANY_H2 = re.compile(r"^##\s+")


def _parse_docs(path: Path) -> Section:
    """
    Read *path* and return a mapping of section label → list of Entry objects.

    Only rows that belong to a *discriminant* table (one whose header row
    contains keywords like "Error Code", "Discriminant", "Code | Variant",
    etc.) are collected.  Plain enum-value tables such as the StreamKind table
    are silently skipped even when they appear inside a recognised section.

    Raises FileNotFoundError / PermissionError if the file cannot be read.
    Raises ValueError if no recognisable discriminant table is found.
    """
    text = path.read_text(encoding="utf-8")
    lines = text.splitlines()

    sections: Section = {}
    current_label: str | None = None
    # True while we are inside a table whose header identified it as a
    # discriminant table (resets to False at the first non-table line after
    # the separator row).
    in_disc_table: bool = False
    # Accumulates the header row text so we can test it against keywords once
    # we see the separator row that confirms it is a table.
    pending_header: str = ""

    for lineno, raw in enumerate(lines, start=1):
        line = raw.strip()

        # ── Detect section boundaries ───────────────────────────────────
        matched_section = False
        for label, pat in _SECTION_PATTERNS:
            if pat.search(line):
                current_label = label
                if current_label not in sections:
                    sections[current_label] = []
                in_disc_table = False
                pending_header = ""
                matched_section = True
                break

        if not matched_section:
            # Reset on an unrecognised H2 boundary so we don't collect rows
            # from a completely unrelated table.
            if _ANY_H2.match(line) and current_label is not None:
                known = any(pat.search(line) for _, pat in _SECTION_PATTERNS)
                if not known:
                    current_label = None
                    in_disc_table = False
                    pending_header = ""

        if current_label is None:
            continue

        # ── Track table header / separator / end ────────────────────────
        if line.startswith("|"):
            if _TABLE_SEP.match(line):
                # Separator row confirms the preceding pipe-row was a header.
                # Decide whether this is a discriminant table.
                in_disc_table = bool(_DISC_HEADER_KEYWORDS.search(pending_header))
                pending_header = ""
                continue
            else:
                # Either a header candidate or a data row.
                if not in_disc_table:
                    # Save as a potential header; we'll confirm on the separator.
                    pending_header = line
        else:
            # Non-pipe line ends the current table.
            in_disc_table = False
            pending_header = ""
            continue

        if not in_disc_table:
            continue

        # ── Try to parse a discriminant data row ────────────────────────
        m = _ROW_DISC_FIRST.match(line)
        if m:
            code = int(m.group(1))
            name = m.group(2)
            sections[current_label].append(Entry(code=code, name=name, line_no=lineno))
            continue

        m = _ROW_NAME_FIRST.match(line)
        if m:
            name = m.group(1)
            code = int(m.group(2))
            sections[current_label].append(Entry(code=code, name=name, line_no=lineno))

    if not sections:
        raise ValueError(
            f"No discriminant tables found in {path}. "
            "Check that the section headings match the expected patterns."
        )

    return sections


# ---------------------------------------------------------------------------
# Analysis
# ---------------------------------------------------------------------------

def _find_intra_collisions(label: str, entries: List[Entry]) -> List[str]:
    """
    Return a list of human-readable messages for every case where two
    DIFFERENT variant names share the same numeric code within one section.
    """
    by_code: Dict[int, List[Entry]] = defaultdict(list)
    for e in entries:
        by_code[e.code].append(e)

    messages = []
    for code, group in sorted(by_code.items()):
        unique_names = {e.name for e in group}
        if len(unique_names) > 1:
            names_str = ", ".join(sorted(unique_names))
            locations = ", ".join(f"line {e.line_no}" for e in group)
            messages.append(
                f"  [INTRA-COLLISION] {label}: code {code} → {{{names_str}}} ({locations})"
            )
    return messages


def _find_cross_collisions(sections: Section) -> List[str]:
    """
    Return a list of human-readable messages for every numeric code that
    appears in more than one section.
    """
    code_to_sections: Dict[int, List[Tuple[str, str]]] = defaultdict(list)
    for label, entries in sections.items():
        seen_in_this_section: Dict[int, str] = {}
        for e in entries:
            # Use the first name encountered for each code in a section.
            if e.code not in seen_in_this_section:
                seen_in_this_section[e.code] = e.name
        for code, name in seen_in_this_section.items():
            code_to_sections[code].append((label, name))

    messages = []
    for code, section_entries in sorted(code_to_sections.items()):
        if len(section_entries) > 1:
            detail = "; ".join(f"{lbl}::{nm}" for lbl, nm in section_entries)
            messages.append(
                f"  [CROSS-SECTION]   code {code} appears in multiple sections: {detail}"
            )
    return messages


def _find_ordering_issues(label: str, entries: List[Entry]) -> List[str]:
    """
    Return messages for any out-of-order discriminant within a section.
    """
    messages = []
    prev_code: int | None = None
    for e in entries:
        if prev_code is not None and e.code < prev_code:
            messages.append(
                f"  [OUT-OF-ORDER]    {label}: code {e.code} (`{e.name}`, line {e.line_no}) "
                f"follows code {prev_code}"
            )
        prev_code = e.code
    return messages


# ---------------------------------------------------------------------------
# Main
# ---------------------------------------------------------------------------

def _build_arg_parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(
        description=__doc__,
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    p.add_argument(
        "--docs",
        metavar="PATH",
        default=None,
        help="Path to docs/error.md (auto-detected if not given)",
    )
    return p


def main(argv: list[str] | None = None) -> int:
    args = _build_arg_parser().parse_args(argv)

    # ── Locate docs/error.md ────────────────────────────────────────────
    if args.docs:
        docs_path = Path(args.docs)
    else:
        # Walk up from this script's location until we find docs/error.md.
        here = Path(__file__).resolve().parent
        candidate = here.parent / "docs" / "error.md"
        if not candidate.exists():
            # Try cwd as a fallback.
            candidate = Path("docs") / "error.md"
        docs_path = candidate

    print(f"Auditing discriminant tables in: {docs_path}\n")

    try:
        sections = _parse_docs(docs_path)
    except FileNotFoundError:
        print(f"ERROR: File not found: {docs_path}", file=sys.stderr)
        return 2
    except ValueError as exc:
        print(f"ERROR: {exc}", file=sys.stderr)
        return 2

    # ── Report what was found ────────────────────────────────────────────
    print("Sections detected:")
    for label, entries in sections.items():
        codes = sorted({e.code for e in entries})
        print(f"  {label}: {len(entries)} entries, codes {codes[0]}–{codes[-1]}")
    print()

    intra_msgs: List[str] = []
    order_msgs: List[str] = []

    for label, entries in sections.items():
        intra_msgs.extend(_find_intra_collisions(label, entries))
        order_msgs.extend(_find_ordering_issues(label, entries))

    cross_msgs = _find_cross_collisions(sections)

    # ── Shared-decoder finding ───────────────────────────────────────────
    print("=" * 70)
    print("SHARED-DECODER FINDING")
    print("=" * 70)
    print(
        "ContractError (stream), FactoryError (factory), and GovernanceError\n"
        "(governance) are three *independent* Soroban #[contracterror] enums.\n"
        "Each lives in a separate contract and is decoded from a distinct\n"
        "on-chain invocation context (stream contract vs. factory contract vs.\n"
        "governance contract).  A well-structured off-chain client (wallet,\n"
        "indexer, SDK) MUST route decoding based on the *invoked contract\n"
        "address* before interpreting the numeric code.  As long as that\n"
        "routing is in place, cross-section code overlap is harmless.\n"
        "\n"
        "RISK: If any off-chain component ever merges all three error\n"
        "namespaces into one shared lookup table (e.g. a single numeric-code\n"
        "→ message map used for all three contracts), cross-section overlaps\n"
        "become silent misclassifications.  The cross-section findings below\n"
        "should be reviewed whenever a new shared-decoder is written.\n"
    )
    print()

    # ── Print findings ───────────────────────────────────────────────────
    if order_msgs:
        print("OUT-OF-ORDER ENTRIES (documentation maintenance warning):")
        for m in order_msgs:
            print(m)
        print()

    if cross_msgs:
        print("CROSS-SECTION OVERLAPS (warning — see shared-decoder note above):")
        for m in cross_msgs:
            print(m)
        print()
    else:
        print("No cross-section numeric overlaps detected.\n")

    if intra_msgs:
        print("INTRA-SECTION COLLISIONS (ERROR — same code, different name, same enum):")
        for m in intra_msgs:
            print(m)
        print()
        print(
            "ACTION REQUIRED: The collisions above indicate that the docs/error.md\n"
            "table assigns the same discriminant to two different variants within\n"
            "the SAME enum.  Fix the table (and/or the source enum) before merging."
        )
        return 1

    print("No intra-section collisions found.  Discriminant tables are internally consistent.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
