#!/usr/bin/env python3
"""Count #[test] attributes in workspace contract Rust sources."""

from __future__ import annotations

import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]
TEST_ATTR = re.compile(r"#\[test\]")


def contract_rust_files() -> list[Path]:
    contracts = REPO_ROOT / "contracts"
    paths: list[Path] = []
    paths.extend(sorted(contracts.glob("*/tests/*.rs")))
    paths.extend(sorted(contracts.glob("*/src/*.rs")))
    return paths


def count_tests() -> tuple[int, dict[str, int]]:
    by_crate: dict[str, int] = {}
    total = 0
    contracts = REPO_ROOT / "contracts"
    for path in contract_rust_files():
        text = path.read_text(encoding="utf-8")
        n = len(TEST_ATTR.findall(text))
        if n == 0:
            continue
        crate = path.relative_to(contracts).parts[0]
        by_crate[crate] = by_crate.get(crate, 0) + n
        total += n
    return total, by_crate


def main() -> int:
    total, by_crate = count_tests()
    print(f"Total #[test] attributes: {total}")
    for crate in sorted(by_crate):
        print(f"  {crate}: {by_crate[crate]}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())

