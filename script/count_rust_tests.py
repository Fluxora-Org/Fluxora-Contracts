#!/usr/bin/env python3
"""Count and report Rust #[test] functions across the workspace.

Outputs a summary suitable for GitHub Actions step summaries.
"""

import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent


def count_tests_in_file(filepath: Path) -> list[str]:
    """Find all #[test] function names in a Rust file."""
    content = filepath.read_text(encoding="utf-8")
    # Match #[test] followed (possibly after other attributes) by fn name
    pattern = re.compile(
        r"#\[test\]\s*(?:#\[.+?\]\s*)*(?:pub\s+)?fn\s+(\w+)",
        re.MULTILINE,
    )
    return pattern.findall(content)


def main() -> int:
    test_dir = REPO_ROOT / "contracts"
    if not test_dir.exists():
        print("No contracts/ directory found")
        return 0

    total = 0
    files_with_tests = 0

    for rs_file in sorted(test_dir.rglob("*.rs")):
        tests = count_tests_in_file(rs_file)
        if tests:
            files_with_tests += 1
            rel = rs_file.relative_to(REPO_ROOT)
            print(f"  {rel}: {len(tests)} test(s)")
            for t in tests:
                print(f"    - {t}")
            total += len(tests)

    print(f"\nTotal: {total} test(s) across {files_with_tests} file(s)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
