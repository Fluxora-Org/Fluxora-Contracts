#!/usr/bin/env python3
"""Check snapshot security-field diffs between PR and base branch.

Parses snapshot JSON files changed in a PR and exits 1 if any
security-relevant field (auth, events, error codes, storage) was altered.
"""

import argparse
import json
import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent

# Security-relevant fields in snapshot JSON files
SECURITY_FIELDS = {
    "auth", "events", "error_code", "error", "storage_keys",
    "storage", "contract_errors", "topics",
}


def get_changed_snapshots(base: str) -> list[Path]:
    """Return snapshot files changed between base and HEAD."""
    result = subprocess.run(
        ["git", "diff", "--name-only", f"{base}...HEAD"],
        capture_output=True,
        text=True,
        cwd=REPO_ROOT,
    )
    if result.returncode != 0:
        print(f"WARNING: git diff failed: {result.stderr}")
        return []

    changed = []
    for line in result.stdout.splitlines():
        path = REPO_ROOT / line.strip()
        if path.suffix == ".json" and "test_snapshots" in str(path):
            changed.append(path)
    return changed


def check_snapshot_security_fields(snapshot_path: Path, base: str) -> list[str]:
    """Check if security fields changed in a snapshot file."""
    issues = []

    try:
        rel = snapshot_path.relative_to(REPO_ROOT)
    except ValueError:
        issues.append(f"{snapshot_path.name}: path is not under repository root")
        return issues

    try:
        # Get the base version
        base_result = subprocess.run(
            ["git", "show", f"{base}:{rel}"],
            capture_output=True,
            text=True,
            cwd=REPO_ROOT,
        )

        if base_result.returncode != 0:
            # New file, no base version
            return issues

        base_data = json.loads(base_result.stdout)
        head_data = json.loads(snapshot_path.read_text())

        for field in SECURITY_FIELDS:
            if field in base_data and field in head_data:
                if base_data[field] != head_data[field]:
                    issues.append(f"{rel}: field '{field}' changed")
            elif field in base_data and field not in head_data:
                issues.append(f"{rel}: field '{field}' removed")
            elif field not in base_data and field in head_data:
                issues.append(f"{rel}: field '{field}' added")
    except (json.JSONDecodeError, OSError) as e:
        issues.append(f"{snapshot_path.name}: error: {e}")

    return issues


def main() -> int:
    parser = argparse.ArgumentParser(description="Check snapshot security diffs")
    parser.add_argument("--base", required=True, help="Base ref to compare against")
    args = parser.parse_args()

    snapshots = get_changed_snapshots(args.base)
    if not snapshots:
        print("No snapshot files changed.")
        return 0

    print(f"Checking {len(snapshots)} changed snapshot file(s)...")
    all_issues = []

    for snap in snapshots:
        issues = check_snapshot_security_fields(snap, args.base)
        all_issues.extend(issues)

    if all_issues:
        print("FAIL: Security-relevant snapshot changes detected:")
        for issue in all_issues:
            print(f"  - {issue}")
        print("\nThese changes require mandatory extra review before merging.")
        return 1

    print("OK: No security-relevant snapshot changes detected.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
