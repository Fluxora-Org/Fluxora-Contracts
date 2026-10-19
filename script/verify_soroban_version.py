#!/usr/bin/env python3
"""Verify the Soroban SDK version mirrors match the Cargo.toml pin."""

import re
import sys
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parent.parent


def check_versions(repo_root: Path = REPO_ROOT) -> list[str]:
    cargo_content = (repo_root / "Cargo.toml").read_text(encoding="utf-8")
    workspace_dependencies = re.search(
        r"(?ms)^\[workspace\.dependencies\]\s*(.*?)(?=^\[|\Z)",
        cargo_content,
    )
    sdk_pins = (
        re.findall(
            r'^\s*soroban-sdk\s*=\s*"([^"]+)"\s*$',
            workspace_dependencies.group(1),
            re.M,
        )
        if workspace_dependencies
        else []
    )
    if len(sdk_pins) != 1:
        return ["Cargo.toml must define exactly one [workspace.dependencies].soroban-sdk pin"]

    cargo_version = sdk_pins[0]
    cargo_major = cargo_version.split(".", 1)[0]
    issues = []

    text_version = (repo_root / "soroban_version.txt").read_text(encoding="utf-8").strip()
    if text_version != cargo_version:
        issues.append(
            f"soroban_version.txt is {text_version!r}; expected {cargo_version!r} from Cargo.toml"
        )

    toolchain_content = (repo_root / "rust-toolchain.toml").read_text(encoding="utf-8")
    toolchain_majors = re.findall(
        r"^\s*#\s*soroban-sdk\s+(\d+)\.x\b", toolchain_content, re.M
    )
    if toolchain_majors != [cargo_major]:
        actual = ", ".join(f"{major}.x" for major in toolchain_majors) or "missing"
        issues.append(
            f"rust-toolchain.toml Soroban SDK record is {actual}; expected {cargo_major}.x "
            f"from Cargo.toml"
        )

    return issues


def main() -> int:
    try:
        issues = check_versions()
    except OSError as exc:
        print(f"Soroban version check failed: {exc}", file=sys.stderr)
        return 1

    if issues:
        print("Soroban SDK version mismatch:", file=sys.stderr)
        for issue in issues:
            print(f"- {issue}", file=sys.stderr)
        print(
            "Cargo.toml [workspace.dependencies].soroban-sdk is authoritative. "
            "Update that pin, then run python3 script/update_soroban_version.py.",
            file=sys.stderr,
        )
        return 1

    print("Soroban SDK version records agree with Cargo.toml")
    return 0


if __name__ == "__main__":
    sys.exit(main())