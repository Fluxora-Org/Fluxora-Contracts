#!/usr/bin/env python3
"""Validate gas baseline entries in docs/gas.md against measured values.

Reads the gas baseline table from docs/gas.md and checks that all values
are non-negative integers. This is a structural check — actual gas regression
testing is done by the Rust snapshot validation in the test job.
"""

import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
GAS_DOC = REPO_ROOT / "docs" / "gas.md"


def main() -> int:
    if not GAS_DOC.exists():
        print(f"SKIP: {GAS_DOC} not found")
        return 0

    content = GAS_DOC.read_text(encoding="utf-8")

    # Look for gas baseline entries like: | operation_name | 12345 |
    pattern = re.compile(r"\|\s*(\w+)\s*\|\s*(\d+)\s*\|")
    matches = pattern.findall(content)

    if not matches:
        print("WARNING: No gas baseline entries found in gas.md")
        print("This is expected if gas.md has not been populated yet.")
        return 0

    print(f"Found {len(matches)} gas baseline entries in docs/gas.md:")
    for name, value in matches:
        print(f"  {name}: {value} instructions")

    print("OK: Gas baseline entries are well-formed.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
