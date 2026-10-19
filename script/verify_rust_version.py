#!/usr/bin/env python3
"""Verify the installed Rust toolchain version matches rust-toolchain.toml."""

import os
import re
import subprocess
import sys
from pathlib import Path

try:
    import tomllib
except ModuleNotFoundError:  # pragma: no cover
    tomllib = None

REPO_ROOT = Path(__file__).resolve().parent.parent
TOOLCHAIN_FILE = REPO_ROOT / "rust-toolchain.toml"


def _parse_toml_simple(content: str):
    data = {}
    section = None
    for raw in content.splitlines():
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        if line.startswith("[") and line.endswith("]"):
            section = line[1:-1].strip()
            data.setdefault(section, {})
            continue
        if "=" not in line:
            continue
        key, value = [part.strip() for part in line.split("=", 1)]
        value = value.strip()
        if value.startswith('"') and value.endswith('"'):
            value = value[1:-1]
        elif value.startswith("[") and value.endswith("]"):
            inner = value[1:-1].strip()
            items = []
            if inner:
                for item in inner.split(","):
                    item = item.strip().strip('"').strip("'")
                    if item:
                        items.append(item)
            value = items
        if section is not None:
            data[section][key] = value
    return data


def _load_toolchain(path: Path | str | None = None):
    file_path = Path(path) if path is not None else TOOLCHAIN_FILE
    text = file_path.read_text(encoding="utf-8")
    if tomllib is not None:
        try:
            return tomllib.loads(text)
        except Exception:
            pass
    return _parse_toml_simple(text)


def pinned_channel(toolchain_file: Path | str | None = None) -> str | None:
    """Extract the channel string from rust-toolchain.toml."""
    file_path = Path(toolchain_file) if toolchain_file is not None else TOOLCHAIN_FILE
    data = _load_toolchain(file_path)
    channel = data.get("toolchain", {}).get("channel")
    if channel is None:
        return None
    return str(channel)


def parse_rustc_version(output: str) -> str:
    match = re.search(r"rustc\s+(\d+\.\d+\.\d+)", output)
    if not match:
        raise ValueError("could not parse rustc version output")
    return match.group(1)


def rustc_version() -> str:
    raw = os.environ.get("RUSTC_VERSION_OUTPUT")
    if raw:
        return parse_rustc_version(raw)
    result = subprocess.run(["rustc", "--version"], capture_output=True, text=True, check=False)
    if result.returncode != 0:
        raise RuntimeError("rustc --version failed")
    return parse_rustc_version(result.stdout)


def pinned_targets(toolchain_file: Path | str | None = None):
    data = _load_toolchain(toolchain_file)
    targets = data.get("toolchain", {}).get("targets")
    if targets is None:
        return []
    if not isinstance(targets, list):
        raise ValueError("invalid targets in rust-toolchain.toml")
    return [str(v) for v in targets]


def pinned_components(toolchain_file: Path | str | None = None):
    data = _load_toolchain(toolchain_file)
    components = data.get("toolchain", {}).get("components")
    if components is None:
        return []
    if not isinstance(components, list):
        raise ValueError("invalid components in rust-toolchain.toml")
    return [str(v) for v in components]


def _installed_rustup_lines(variable: str, command: list[str]) -> list[str]:
    override = os.environ.get(variable)
    if override is not None:
        return override.splitlines()
    result = subprocess.run(
        ["rustup", *command, "--installed"],
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode != 0:
        raise RuntimeError(f"rustup {' '.join(command)} --installed failed")
    return result.stdout.splitlines()


def main() -> int:
    try:
        expected = pinned_channel()
        if expected is None:
            print("::error:: rust-toolchain.toml is missing [toolchain].channel", file=sys.stderr)
            return 1

        installed = rustc_version()
        if installed != expected:
            print(f"Rust version mismatch: expected {expected}, got {installed}", file=sys.stderr)
            return 1
        print(f"Rust version matches pinned {expected}")

        installed_targets = _installed_rustup_lines(
            "RUSTUP_TARGET_LIST_OUTPUT", ["target", "list"]
        )
        required_targets = pinned_targets()
        missing_targets = [t for t in required_targets if t not in installed_targets]
        if missing_targets:
            print(f"Missing required targets: {', '.join(missing_targets)}", file=sys.stderr)
            return 1
        print("Installed targets match requirements")

        installed_components = _installed_rustup_lines(
            "RUSTUP_COMPONENT_LIST_OUTPUT", ["component", "list"]
        )
        required_components = pinned_components()
        missing_components = [
            component
            for component in required_components
            if not any(
                installed == component or installed.startswith(f"{component}-")
                for installed in installed_components
            )
        ]
        if missing_components:
            print(f"Missing required components: {', '.join(missing_components)}", file=sys.stderr)
            return 1
        print("Installed components match requirements")
        return 0
    except Exception as exc:
        print(f"::error:: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
