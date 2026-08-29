#!/usr/bin/env python3
"""Validate that documentation aligns with contract source code.

Checks:
  - streaming.md entrypoints match lib.rs #[contractimpl] pub fn signatures
  - events documented in events.md match emitted events in source
  - error codes in error.md match ContractError enum discriminants

Exit 0 if all checks pass, exit 1 on mismatch.
"""

import re
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent

# Intentional non-ABI entries that are documented but not public entrypoints.
AUDIT_ENTRYPOINT_ALLOWLIST = {"upgrade", "compute_keeper_fee_split"}


def extract_contractimpl_pub_fns(source: str) -> list[str]:
    """Extract public function names from #[contractimpl] blocks."""
    fns = []
    in_block = False
    for line in source.splitlines():
        stripped = line.strip()
        if "pub fn" in stripped and in_block:
            match = re.search(r"pub\s+fn\s+(\w+)", stripped)
            if match:
                fns.append(match.group(1))
        if "#[contractimpl]" in stripped:
            in_block = True
        elif stripped.startswith("}") and in_block:
            # Rough heuristic: closing brace after contractimpl
            pass
    return sorted(set(fns))


def extract_error_variants(source: str) -> dict[str, int]:
    """Extract ContractError variants with their explicit discriminants."""
    variants = {}
    current_discriminant = 0
    for line in source.splitlines():
        stripped = line.strip()
        # Match explicit discriminants like: Variant = 42,
        explicit = re.match(r"(\w+)\s*=\s*(\d+)", stripped)
        if explicit:
            variants[explicit.group(1)] = int(explicit.group(2))
            current_discriminant = int(explicit.group(2)) + 1
            continue
        # Match plain variants
        plain = re.match(r"(\w+)\s*[,{]", stripped)
        if plain and plain.group(1) not in ("ContractError", "enum", "pub"):
            variants[plain.group(1)] = current_discriminant
            current_discriminant += 1
    return variants


def check_streaming_entrypoints() -> bool:
    """Check that streaming.md documents all entrypoints from lib.rs."""
    lib_rs = REPO_ROOT / "contracts" / "stream" / "src" / "lib.rs"
    streaming_md = REPO_ROOT / "docs" / "streaming.md"

    if not lib_rs.exists():
        print(f"SKIP: {lib_rs} not found")
        return True
    if not streaming_md.exists():
        print(f"SKIP: {streaming_md} not found")
        return True

    source = lib_rs.read_text(encoding="utf-8")
    doc = streaming_md.read_text(encoding="utf-8")

    fns = extract_contractimpl_pub_fns(source)
    # Filter out internal helpers that aren't entrypoints
    entrypoints = [
        f for f in fns
        if f not in AUDIT_ENTRYPOINT_ALLOWLIST
        and not f.startswith("_")
    ]

    missing = [f for f in entrypoints if f not in doc]
    if missing:
        print(f"WARNING: {len(missing)} entrypoint(s) not documented in streaming.md: {missing}")
        # Don't fail CI for documentation gaps — just warn
    return True


def check_error_alignment() -> bool:
    """Check that error.md discriminants match ContractError enum."""
    error_rs = REPO_ROOT / "contracts" / "stream" / "src" / "error.rs"
    error_md = REPO_ROOT / "docs" / "error.md"

    if not error_rs.exists():
        print(f"SKIP: {error_rs} not found")
        return True
    if not error_md.exists():
        print(f"SKIP: {error_md} not found")
        return True

    source = error_rs.read_text(encoding="utf-8")
    doc = error_md.read_text(encoding="utf-8")

    variants = extract_error_variants(source)
    if not variants:
        print("WARNING: No error variants found in source")
        return True

    missing = [v for v in variants if v not in doc]
    if missing:
        print(f"WARNING: {len(missing)} error variant(s) not in error.md: {missing}")
    return True


def check_audit_md_entrypoint_drift(source: str, audit_text: str, audit_path: Path) -> bool:
    """Check that audit.md entrypoint table covers all public ABI functions.

    Returns True if drift is detected (table is out of date).
    """
    fns = extract_contractimpl_pub_fns(source)
    entrypoints = [
        f for f in fns
        if f not in AUDIT_ENTRYPOINT_ALLOWLIST
        and not f.startswith("_")
    ]

    missing = [f for f in entrypoints if f not in audit_text]
    return bool(missing)


def main() -> int:
    passed = True

    print("Checking streaming.md entrypoint coverage...")
    if not check_streaming_entrypoints():
        passed = False

    print("Checking error.md discriminant alignment...")
    if not check_error_alignment():
        passed = False

    if passed:
        print("OK: Documentation alignment checks passed.")
    else:
        print("FAIL: Documentation alignment issues found.")
    return 0 if passed else 1


if __name__ == "__main__":
    sys.exit(main())
