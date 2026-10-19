#!/usr/bin/env python3
"""Check snapshot security-field diffs between PR and base branch.

Parses snapshot JSON files changed in a PR and exits 1 if any
security-relevant field (auth, events, error codes, storage) was altered.
"""

import argparse
import json
import re
import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent

# Security-relevant fields in snapshot JSON files.
SECURITY_FIELDS = {
    "auth",
    "auths",
    "require_auth",
    "signatures",
    "events",
    "event",
    "topic",
    "topics",
    "data",
    "error",
    "error_code",
    "ContractError",
    "storage",
    "storage_keys",
    "state",
    "DataKey",
    "contract_errors",
}


def is_security_relevant(path: str) -> bool:
    """Return True when a JSON diff path is security relevant."""
    if not path:
        return False
    components = {
        component.lower()
        for component in re.findall(r"[A-Za-z_][A-Za-z_0-9]*", path)
    }
    return any(field.lower() in components for field in SECURITY_FIELDS)


def _join_path(prefix: str, key: str) -> str:
    if not prefix:
        return key
    return f"{prefix}.{key}"


def get_diff_paths(old, new, prefix: str = ""):
    """Recursively compute a list of changed JSON paths."""
    if type(old) != type(new):
        return [prefix] if prefix else [""]
    if old == new:
        return []

    if isinstance(old, dict):
        diffs = []
        keys = set(old) | set(new)
        for key in sorted(keys):
            if key not in old:
                diffs.append(_join_path(prefix, key) if prefix else key)
            elif key not in new:
                diffs.append(_join_path(prefix, key) if prefix else key)
            else:
                diffs.extend(get_diff_paths(old[key], new[key], _join_path(prefix, key) if prefix else key))
        return diffs

    if isinstance(old, list):
        diffs = []
        length = min(len(old), len(new))
        for idx in range(length):
            diffs.extend(get_diff_paths(old[idx], new[idx], f"{prefix}[{idx}]" if prefix else f"[{idx}]"))
        if len(old) != len(new):
            diffs.append(prefix if prefix else "")
        return diffs

    return [prefix] if prefix else [""]


def get_changed_files(base: str, head=None) -> list[str]:
    """Return JSON snapshot files changed between base and head."""
    try:
        if head is None:
            cmd = ["git", "diff", "--name-only", base]
        else:
            cmd = ["git", "diff", "--name-only", base, head]
        out = subprocess.check_output(cmd, cwd=REPO_ROOT, text=False)
    except (subprocess.CalledProcessError, OSError):
        return []

    files = []
    for line in out.decode("utf-8", errors="replace").splitlines():
        path = line.strip()
        if path.endswith(".json") and "test_snapshots" in path:
            files.append(path)
    return files


def get_file_content(commit, path):
    """Return file content from git history or disk."""
    if commit is not None:
        try:
            out = subprocess.check_output(["git", "show", f"{commit}:{path}"])
            return out.decode("utf-8", errors="replace")
        except (subprocess.CalledProcessError, OSError, ValueError):
            return None
    full = REPO_ROOT / path
    if not full.exists():
        return None
    return full.read_text(encoding="utf-8")


def _safe_json(text):
    if text is None:
        return {}
    try:
        return json.loads(text)
    except (TypeError, ValueError):
        return {}


def main() -> int:
    parser = argparse.ArgumentParser(description="Check snapshot security diffs")
    parser.add_argument("--base", default="HEAD", help="Base ref to compare against")
    parser.add_argument("--head", default=None, help="Optional head ref to compare against")
    args = parser.parse_args()

    changed = get_changed_files(args.base, args.head)
    if not changed:
        print("No snapshot JSON files changed.")
        return 0

    print(f"Checking {len(changed)} changed snapshot file(s)...")
    found_security = False
    messages = []

    for rel_path in changed:
        old_text = get_file_content(args.base, rel_path)
        new_text = get_file_content(args.head, rel_path)
        old_data = _safe_json(old_text)
        new_data = _safe_json(new_text)

        diffs = get_diff_paths(old_data, new_data)
        security_paths = [d for d in diffs if is_security_relevant(d)]
        if security_paths:
            found_security = True
            messages.append(f"{rel_path}: {', '.join(sorted(set(security_paths)))}")
        elif diffs:
            print(
                f"[INFO] Changes in {rel_path}: {', '.join(sorted(set(diffs)))}; "
                "none are security-relevant."
            )

    if found_security:
        print("Security-relevant fields changed:")
        for msg in messages:
            print(f"  - {msg}")
        print("\nMandatory extra review required.")
        return 1

    print("No security-relevant snapshot changes detected.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
