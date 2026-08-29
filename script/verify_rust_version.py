#!/usr/bin/env python3
"""Verify the installed Rust toolchain version matches rust-toolchain.toml."""

import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent


def parse_toolchain_toml(channel: str | None = None) -> str | None:
    """Extract the channel from rust-toolchain.toml."""
    toml_path = REPO_ROOT / "rust-toolchain.toml"
    if not toml_path.exists():
        return channel
    for line in toml_path.read_text().splitlines():
        stripped = line.strip()
        if stripped.startswith("channel") and "=" in stripped:
            value = stripped.split("=", 1)[1].strip().strip('"').strip("'")
            return value
    return channel


def get_installed_version() -> str | None:
    """Return the installed rustc version string, or None if rustc is missing."""
    try:
        result = subprocess.run(
            ["rustc", "--version"],
            capture_output=True,
            text=True,
            check=True,
        )
        return result.stdout.strip()
    except FileNotFoundError:
        return None


def main() -> int:
    expected = parse_toolchain_toml()
    installed = get_installed_version()

    if installed is None:
        print("WARNING: rustc not found; skipping version check.")
        return 0

    if expected is None:
        print(f"rust-toolchain.toml not found; installed: {installed}")
        print("WARNING: No pinned toolchain to verify against.")
        return 0

    # expected is like "1.97.1"; installed is like "rustc 1.97.1 ( ..."
    if expected in installed:
        print(f"OK: installed rustc matches pinned channel {expected} — {installed}")
        return 0

    print(f"MISMATCH: expected channel {expected}, got {installed}")
    print("Install the correct toolchain with: rustup install " + expected)
    return 1


if __name__ == "__main__":
    sys.exit(main())
