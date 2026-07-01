#!/usr/bin/env python3
"""Verify rustc matches the pinned rust-toolchain.toml channel."""

from __future__ import annotations

import os
import re
import subprocess
import sys
from pathlib import Path

try:
    import tomllib
except ModuleNotFoundError:  # pragma: no cover - Python < 3.11 fallback
    import tomli as tomllib  # type: ignore[no-redef]


REPO_ROOT = Path(__file__).resolve().parents[1]
TOOLCHAIN_FILE = REPO_ROOT / "rust-toolchain.toml"


def pinned_channel(toolchain_file: Path = TOOLCHAIN_FILE) -> str:
    data = tomllib.loads(toolchain_file.read_text(encoding="utf-8"))
    channel = data.get("toolchain", {}).get("channel")
    if not isinstance(channel, str) or not channel:
        raise ValueError(f"missing [toolchain].channel in {toolchain_file}")
    return channel


def parse_rustc_version(version_output: str) -> str:
    match = re.match(r"^rustc\s+([^\s]+)", version_output.strip())
    if not match:
        raise ValueError(f"could not parse rustc version from: {version_output!r}")
    return match.group(1)


def rustc_version() -> str:
    override = os.environ.get("RUSTC_VERSION_OUTPUT")
    if override:
        return parse_rustc_version(override)

    completed = subprocess.run(
        ["rustc", "--version"],
        check=True,
        capture_output=True,
        text=True,
    )
    return parse_rustc_version(completed.stdout)


def main() -> int:
    try:
        expected = pinned_channel()
        actual = rustc_version()
    except Exception as exc:
        print(f"::error::{exc}", file=sys.stderr)
        return 1

    if actual != expected:
        print(
            f"::error::Rust version mismatch: expected {expected}, got {actual}",
            file=sys.stderr,
        )
        return 1

    print(f"Rust version matches pinned {expected}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
