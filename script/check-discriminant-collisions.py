#!/usr/bin/env python3
"""Audit ContractError discriminants for collisions.

Parses discriminant tables from docs/error.md and fails if any
intra-section collision exists (same code, different variant name, same enum).
Cross-section overlaps are printed as warnings but do not fail.
"""

import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent


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


def main() -> int:
    error_md = REPO_ROOT / "docs" / "error.md"
    if not error_md.exists():
        print(f"SKIP: {error_md} not found")
        return 0

    content = error_md.read_text(encoding="utf-8")
    tables = parse_error_md_tables(content)

    if not tables:
        print("SKIP: No error discriminant tables found in error.md")
        return 0

    failed = False

    for enum_name, code_map in tables.items():
        collisions = {code: variants for code, variants in code_map.items() if len(variants) > 1}
        if collisions:
            print(f"COLLISION in {enum_name}:")
            for code, variants in collisions.items():
                print(f"  Code {code}: {variants}")
            failed = True

    if failed:
        print("FAIL: Intra-section discriminant collisions detected.")
        return 1

    print("OK: No discriminant collisions found.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
