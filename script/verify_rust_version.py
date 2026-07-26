#!/usr/bin/env python3
"""Verify rustc matches the pinned rust-toolchain.toml channel."""

from __future__ import annotations

import os
import re
import subprocess
import sys
from pathlib import Path

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


def pinned_channel_via_toml(toolchain_file: Path) -> str | None:
    """Best-effort TOML parse with the stdlib `tomllib` (3.11+) or `tomli`.

    Returns `None` when neither library is importable so the caller can fall
    back to the regex parser. We do not raise on ImportError here so the
    legacy Python 3.10 CI runner path stays green.
    """
    try:
        import tomllib as _tomllib  # type: ignore[import-not-found]
    except ImportError:
        try:
            import tomli as _tomllib  # type: ignore[no-redef]
        except ImportError:
            return None
    data = _tomllib.loads(toolchain_file.read_text(encoding="utf-8"))
    channel = data.get("toolchain", {}).get("channel")
    if not isinstance(channel, str) or not channel:
        raise ValueError(f"missing [toolchain].channel in {toolchain_file}")
    return channel


REPO_ROOT = Path(__file__).resolve().parents[1]
TOOLCHAIN_FILE = REPO_ROOT / "rust-toolchain.toml"


def pinned_channel(toolchain_file: Path = TOOLCHAIN_FILE) -> str:
    # Prefer a real TOML parser when a library is available so we get
    # accurate error messages on malformed input. Fall back to the regex
    # parser so the script still works on minimal Python installs (CI uses
    # Python 3.10 without the third-party `tomli` package).
    via_toml = pinned_channel_via_toml(toolchain_file)
    if via_toml is not None:
        return via_toml
    return _channel_from_toolchain_text(toolchain_file.read_text(encoding="utf-8"))


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
