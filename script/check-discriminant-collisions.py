#!/usr/bin/env python3
"""Audit ContractError discriminants for collisions.

Parses discriminant tables from docs/error.md and fails if any
intra-section collision exists (same code, different variant name, same enum).
Cross-section overlaps are printed as warnings but do not fail.
"""

import argparse
import re
import sys
from collections import defaultdict
from typing import NamedTuple
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent


class Entry(NamedTuple):
    code: int
    name: str
    line_no: int


SECTION_PATTERNS = (
    ("ContractError (stream)", re.compile(r"^#{1,4}\s+(?:Error Code Reference Table|`Error`)$", re.I)),
    ("FactoryError (factory)", re.compile(r"^#{1,4}\s+.*FactoryError.*Reference.*Factory", re.I)),
    ("GovernanceError (governance)", re.compile(r"^#{1,4}\s+.*GovernanceError.*Reference", re.I)),
)
ROW_DISC_FIRST = re.compile(r"^\|\s*(\d+)\s*\|\s*`?([A-Za-z_][A-Za-z0-9_]*)`?")
ROW_NAME_FIRST = re.compile(r"^\|\s*`([A-Za-z_][A-Za-z0-9_]*)`\s*\|\s*(\d+)\s*\|")
TABLE_SEPARATOR = re.compile(r"^\|[-|: ]+\|")


def parse_error_md_tables(content: str) -> dict[str, dict[int, list[str]]]:
    """Parse error.md discriminant tables into {enum_name: {code: [variants]}}."""
    tables: dict[str, dict[int, list[str]]] = {}
    current_enum = None

    for line in content.splitlines():
        stripped = line.strip()

        # Detect table headers or section names for enums
        enum_match = re.match(r"^#+\s*(\w+Error)", stripped)
        if enum_match:
            current_enum = enum_match.group(1)
            if current_enum not in tables:
                tables[current_enum] = {}
            continue

        # Parse table rows: | code | variant_name | description |
        row_match = re.match(r"\|\s*(\d+)\s*\|\s*(\w+)\s*\|", stripped)
        if row_match and current_enum:
            code = int(row_match.group(1))
            variant = row_match.group(2)
            tables[current_enum].setdefault(code, []).append(variant)

    return tables


def _parse_docs(path: Path) -> dict[str, list[Entry]]:
    lines = Path(path).read_text(encoding="utf-8").splitlines()
    sections: dict[str, list[Entry]] = {}
    current = None
    header = ""
    in_table = False

    for line_no, raw in enumerate(lines, start=1):
        line = raw.strip()
        matched = False
        for label, pattern in SECTION_PATTERNS:
            if pattern.search(line):
                current = label
                sections.setdefault(label, [])
                header = ""
                in_table = False
                matched = True
                break
        if not matched and line.startswith("## "):
            current = None
            header = ""
            in_table = False
        if current is None:
            continue
        if not line.startswith("|"):
            header = ""
            in_table = False
            continue
        if TABLE_SEPARATOR.match(line):
            in_table = bool(
                re.search(
                    r"error\s*code|discriminant|#\s*\|\s*name|code\s*\|.*variant|variant.*\|\s*code",
                    header,
                    re.I,
                )
            )
            header = ""
            continue
        if not in_table:
            header = line
            continue

        match = ROW_DISC_FIRST.match(line)
        if match:
            sections[current].append(Entry(int(match.group(1)), match.group(2), line_no))
            continue
        match = ROW_NAME_FIRST.match(line)
        if match:
            sections[current].append(Entry(int(match.group(2)), match.group(1), line_no))

    if not sections:
        raise ValueError(f"No discriminant tables found in {path}")
    return sections


def _find_intra_collisions(label: str, entries: list[Entry]) -> list[str]:
    by_code: dict[int, list[Entry]] = defaultdict(list)
    for entry in entries:
        by_code[entry.code].append(entry)
    return [
        f"  [INTRA-COLLISION] {label}: code {code} -> {{{', '.join(sorted({e.name for e in group}))}}} "
        f"({', '.join(f'line {e.line_no}' for e in group)})"
        for code, group in sorted(by_code.items())
        if len({entry.name for entry in group}) > 1
    ]


def _find_cross_collisions(sections: dict[str, list[Entry]]) -> list[str]:
    by_code: dict[int, list[str]] = defaultdict(list)
    for label, entries in sections.items():
        for code in {entry.code for entry in entries}:
            by_code[code].append(label)
    return [
        f"  [CROSS-SECTION] code {code} appears in multiple sections: {', '.join(labels)}"
        for code, labels in sorted(by_code.items())
        if len(labels) > 1
    ]


def _find_ordering_issues(label: str, entries: list[Entry]) -> list[str]:
    messages = []
    previous = None
    for entry in entries:
        if previous is not None and entry.code < previous:
            messages.append(
                f"  [OUT-OF-ORDER] {label}: code {entry.code} at line {entry.line_no} follows {previous}"
            )
        previous = entry.code
    return messages


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description="Audit ABI error discriminants")
    parser.add_argument("--docs", type=Path, default=REPO_ROOT / "docs" / "ABI.md")
    args = parser.parse_args(argv)
    try:
        sections = _parse_docs(args.docs)
    except FileNotFoundError:
        print(f"ERROR: File not found: {args.docs}", file=sys.stderr)
        return 2
    except (OSError, ValueError) as exc:
        print(f"ERROR: {exc}", file=sys.stderr)
        return 2

    print("Discriminant sections:")
    intra = []
    ordering = []
    for label, entries in sections.items():
        print(f"  {label}: {len(entries)} entries")
        intra.extend(_find_intra_collisions(label, entries))
        ordering.extend(_find_ordering_issues(label, entries))
    for message in ordering:
        print(message)
    for message in intra:
        print(message)
    if intra:
        print("ACTION REQUIRED: intra-section discriminant collisions must be fixed.")
    else:
        print("No intra-section collisions found.")

    cross = _find_cross_collisions(sections)
    print("SHARED-DECODER FINDING: equal numeric codes in separate enums can be ambiguous to shared decoders.")
    for message in cross:
        print(message)
    if not cross:
        print("No cross-section numeric overlaps detected.")
    return 1 if intra else 0


if __name__ == "__main__":
    sys.exit(main())
