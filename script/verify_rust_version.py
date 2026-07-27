#!/usr/bin/env python3
"""Verify rustc matches the pinned rust-toolchain.toml channel."""

from __future__ import annotations

import os
import re
import subprocess
import sys
from pathlib import Path

try:
    import tomllib  # type: ignore[import-not-found]
except ImportError:
    tomllib = None  # type: ignore[assignment]

# `rust-toolchain.toml` is a tiny single-key file in this repo, so we read
# `channel = "..."` directly without depending on a TOML library. This keeps
# the script portable across Python versions (3.10 runners used by CI don't
# ship `tomllib` in stdlib) and environments where `tomli` isn't installed.
_CHANNEL_RE = re.compile(
    r'^\s*channel\s*=\s*"(?P<channel>[^"]+)"\s*$',
    re.MULTILINE | re.DOTALL,
)


def _channel_from_toolchain_text(text: str) -> str:
    """Extract `[toolchain].channel` from a `rust-toolchain.toml` body.

    We deliberately accept any channel expression found anywhere in the file
    rather than enforcing a `[toolchain]` section header — `rustup` itself
    tolerates both styles and the contract we verify is "the pinned channel
    matches what `rustc --version` reports", not TOML strictness.
    """
    match = _CHANNEL_RE.search(text)
    if not match:
        raise ValueError(
            "missing `channel = \"...\"` assignment in rust-toolchain.toml"
        )
    channel = match.group("channel").strip()
    if not channel:
        raise ValueError("empty `channel` value in rust-toolchain.toml")
    return channel


REPO_ROOT = Path(__file__).resolve().parents[1]
TOOLCHAIN_FILE = REPO_ROOT / "rust-toolchain.toml"


def _parse_toml_simple(text: str) -> dict:
    """Minimal TOML parser for rust-toolchain.toml — handles the subset we need.

    Extracts [toolchain].channel, [toolchain].targets (array), and
    [toolchain].components (array). Raises ValueError on invalid input.
    """
    result: dict = {"toolchain": {}}
    in_toolchain = False
    for line in text.splitlines():
        stripped = line.strip()
        if not stripped or stripped.startswith("#"):
            continue
        if stripped == "[toolchain]":
            in_toolchain = True
            continue
        if stripped.startswith("[") and stripped.endswith("]"):
            in_toolchain = False
            continue
        if in_toolchain:
            # Parse key = value or key = [array]
            kv_match = re.match(r'^(\w+)\s*=\s*(.+)$', stripped)
            if kv_match:
                key = kv_match.group(1)
                value_str = kv_match.group(2).strip()
                if value_str.startswith("[") and value_str.endswith("]"):
                    # Array value
                    inner = value_str[1:-1].strip()
                    if not inner:
                        result["toolchain"][key] = []
                    else:
                        items = []
                        for item in re.findall(r'"([^"]*)"', inner):
                            items.append(item)
                        result["toolchain"][key] = items
                elif value_str.startswith('"') and value_str.endswith('"'):
                    result["toolchain"][key] = value_str[1:-1]
                else:
                    result["toolchain"][key] = value_str
    return result


def _load_toolchain(toolchain_file: Path) -> dict:
    """Load and parse the toolchain file, preferring tomllib."""
    if tomllib is not None:
        return tomllib.loads(toolchain_file.read_text(encoding="utf-8"))
    return _parse_toml_simple(toolchain_file.read_text(encoding="utf-8"))


def pinned_channel_via_toml(toolchain_file: Path) -> str | None:
    """Best-effort TOML parse with the stdlib `tomllib` (3.11+) or `tomli`.

    Returns `None` when neither library is importable so the caller can fall
    back to the regex parser. We do not raise on ImportError here so the
    legacy Python 3.10 CI runner path stays green.
    """
    try:
        data = _load_toolchain(toolchain_file)
    except Exception:
        return None
    channel = data.get("toolchain", {}).get("channel")
    if not isinstance(channel, str) or not channel:
        return None
    return channel


def pinned_channel(toolchain_file: Path | None = None) -> str:
    if toolchain_file is None:
        toolchain_file = TOOLCHAIN_FILE
    via_toml = pinned_channel_via_toml(toolchain_file)
    if via_toml is not None:
        return via_toml
    return _channel_from_toolchain_text(toolchain_file.read_text(encoding="utf-8"))


def pinned_targets(toolchain_file: Path | None = None) -> list[str]:
    """Return the list of targets declared in rust-toolchain.toml."""
    if toolchain_file is None:
        toolchain_file = TOOLCHAIN_FILE
    data = _load_toolchain(toolchain_file)
    targets = data.get("toolchain", {}).get("targets")
    if targets is None:
        return []
    if not isinstance(targets, list):
        raise ValueError(
            f"invalid targets in {toolchain_file}: expected list, got {type(targets).__name__}"
        )
    return targets


def pinned_components(toolchain_file: Path | None = None) -> list[str]:
    """Return the list of components declared in rust-toolchain.toml."""
    if toolchain_file is None:
        toolchain_file = TOOLCHAIN_FILE
    data = _load_toolchain(toolchain_file)
    components = data.get("toolchain", {}).get("components")
    if components is None:
        return []
    if not isinstance(components, list):
        raise ValueError(
            f"invalid components in {toolchain_file}: expected list, got {type(components).__name__}"
        )
    return components


def installed_targets() -> list[str]:
    """Return the list of installed rustup targets.

    Honours the RUSTUP_TARGET_LIST_OUTPUT env-var override for testability.
    """
    override = os.environ.get("RUSTUP_TARGET_LIST_OUTPUT")
    if override is not None:
        return [t.strip() for t in override.splitlines() if t.strip()]
    completed = subprocess.run(
        ["rustup", "target", "list", "--installed"],
        check=True,
        capture_output=True,
        text=True,
    )
    return [t.strip() for t in completed.stdout.splitlines() if t.strip()]


def installed_components() -> list[str]:
    """Return the list of installed rustup components.

    Honours the RUSTUP_COMPONENT_LIST_OUTPUT env-var override for testability.
    """
    override = os.environ.get("RUSTUP_COMPONENT_LIST_OUTPUT")
    if override is not None:
        return [c.strip() for c in override.splitlines() if c.strip()]
    completed = subprocess.run(
        ["rustup", "component", "list", "--installed"],
        check=True,
        capture_output=True,
        text=True,
    )
    return [c.strip() for c in completed.stdout.splitlines() if c.strip()]


def has_component(installed: list[str], required: str) -> bool:
    """Return true when a required rustup component is installed.

    `rustup component list --installed` commonly prints host-qualified names
    such as `rustfmt-x86_64-unknown-linux-gnu`, while rust-toolchain.toml uses
    bare component names like `rustfmt`. Accept either representation.
    """
    return any(
        component == required or component.startswith(f"{required}-")
        for component in installed
    )


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
        expected = pinned_channel(TOOLCHAIN_FILE)
        actual = rustc_version()
        required_targets = pinned_targets()
        required_components = pinned_components()
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

    # Check required targets
    try:
        required_targets = pinned_targets(TOOLCHAIN_FILE)
    except Exception as exc:
        print(f"::error::{exc}", file=sys.stderr)
        return 1
    if required_targets:
        have_targets = installed_targets()
        missing_targets = [t for t in required_targets if t not in have_targets]
        if missing_targets:
            print(
                f"::error::Missing required targets: {', '.join(missing_targets)}",
                file=sys.stderr,
            )
            return 1
        print("Installed targets match requirements")

    # Check required components
    try:
        required_components = pinned_components(TOOLCHAIN_FILE)
    except Exception as exc:
        print(f"::error::{exc}", file=sys.stderr)
        return 1
    if required_components:
        have_components = installed_components()
        missing_components = [
            c for c in required_components if not has_component(have_components, c)
        ]
        if missing_components:
            print(
                f"::error::Missing required components: {', '.join(missing_components)}",
                file=sys.stderr,
            )
            return 1
        print("Installed components match requirements")

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
