#!/usr/bin/env python3
"""Verify rustc matches the pinned rust-toolchain.toml channel."""

from __future__ import annotations

import os
import re
import subprocess
import sys
from pathlib import Path
from typing import Any

try:
    import tomllib
except ModuleNotFoundError:  # pragma: no cover - Python < 3.11 fallback
    import tomli as tomllib  # type: ignore[no-redef]


REPO_ROOT = Path(__file__).resolve().parents[1]
TOOLCHAIN_FILE = REPO_ROOT / "rust-toolchain.toml"


def _load_toolchain(toolchain_file: Path) -> dict[str, Any]:
    return tomllib.loads(toolchain_file.read_text(encoding="utf-8"))


def pinned_channel(toolchain_file: Path = TOOLCHAIN_FILE) -> str:
    data = _load_toolchain(toolchain_file)
    channel = data.get("toolchain", {}).get("channel")
    if not isinstance(channel, str) or not channel:
        raise ValueError(f"missing [toolchain].channel in {toolchain_file}")
    return channel


def pinned_targets(toolchain_file: Path = TOOLCHAIN_FILE) -> list[str]:
    data = _load_toolchain(toolchain_file)
    targets = data.get("toolchain", {}).get("targets", [])
    if not isinstance(targets, list):
        raise ValueError(f"invalid [toolchain].targets in {toolchain_file}")
    return targets


def pinned_components(toolchain_file: Path = TOOLCHAIN_FILE) -> list[str]:
    data = _load_toolchain(toolchain_file)
    components = data.get("toolchain", {}).get("components", [])
    if not isinstance(components, list):
        raise ValueError(f"invalid [toolchain].components in {toolchain_file}")
    return components


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


def installed_targets() -> list[str]:
    """Return a list of installed rustup targets.
    
    Security: The RUSTUP_TARGET_LIST_OUTPUT environment variable is intended for
    testing purposes only and allows overriding the output of the rustup command.
    """
    override = os.environ.get("RUSTUP_TARGET_LIST_OUTPUT")
    if override is not None:
        return [line.strip() for line in override.splitlines() if line.strip()]

    completed = subprocess.run(
        ["rustup", "target", "list", "--installed"],
        check=True,
        capture_output=True,
        text=True,
    )
    return [line.strip() for line in completed.stdout.splitlines() if line.strip()]


def installed_components() -> list[str]:
    """Return a list of installed rustup components.
    
    Security: The RUSTUP_COMPONENT_LIST_OUTPUT environment variable is intended for
    testing purposes only and allows overriding the output of the rustup command.
    """
    override = os.environ.get("RUSTUP_COMPONENT_LIST_OUTPUT")
    if override is not None:
        return [line.strip() for line in override.splitlines() if line.strip()]

    completed = subprocess.run(
        ["rustup", "component", "list", "--installed"],
        check=True,
        capture_output=True,
        text=True,
    )
    return [line.strip() for line in completed.stdout.splitlines() if line.strip()]


def main() -> int:
    try:
        expected_channel = pinned_channel()
        actual_channel = rustc_version()
        
        expected_targets = set(pinned_targets())
        actual_targets = set(installed_targets())
        
        expected_components = set(pinned_components())
        actual_components = set(installed_components())
    except Exception as exc:
        print(f"::error::{exc}", file=sys.stderr)
        return 1

    errors = 0

    if actual_channel != expected_channel:
        print(
            f"::error::Rust version mismatch: expected {expected_channel}, got {actual_channel}",
            file=sys.stderr,
        )
        errors += 1
    else:
        print(f"Rust version matches pinned {expected_channel}")

    missing_targets = expected_targets - actual_targets
    if missing_targets:
        print(
            f"::error::Missing required targets: {', '.join(sorted(missing_targets))}",
            file=sys.stderr,
        )
        errors += 1
    elif expected_targets:
        print(f"Installed targets match requirements: {', '.join(sorted(expected_targets))}")

    missing_components = expected_components - actual_components
    if missing_components:
        print(
            f"::error::Missing required components: {', '.join(sorted(missing_components))}",
            file=sys.stderr,
        )
        errors += 1
    elif expected_components:
        print(f"Installed components match requirements: {', '.join(sorted(expected_components))}")

    return 1 if errors else 0


if __name__ == "__main__":
    raise SystemExit(main())
