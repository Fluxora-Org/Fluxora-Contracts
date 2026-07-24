#!/usr/bin/env python3
import json
import sys
import subprocess
import os
import argparse

# "Security-relevant" fields based on docs/maintainer-security-checklist.md
SECURITY_FIELDS = {
    'auth', 'auths', 'require_auth', 'signatures', 
    'events', 'topic', 'topics', 'data',
    'error', 'error_code', 'ContractError',
    'storage', 'state', 'DataKey'
}

def get_changed_files(base, head):
    # If head is None, compare base to working tree
    if head:
        cmd = ['git', 'diff', '--name-only', base, head]
    else:
        cmd = ['git', 'diff', '--name-only', base]
    try:
        output = subprocess.check_output(cmd).decode('utf-8')
        return [f for f in output.splitlines() if '/test_snapshots/' in f and f.endswith('.json')]
    except subprocess.CalledProcessError:
        return []

def get_file_content(commit, path):
    # If commit is None, read from local file system
    if not commit:
        if os.path.exists(path):
            with open(path, 'r', encoding='utf-8') as f:
                return f.read()
        return None
    try:
        return subprocess.check_output(['git', 'show', f"{commit}:{path}"]).decode('utf-8')
    except subprocess.CalledProcessError:
        return None

def get_diff_paths(old, new, path=""):
    """
    Recursively compares old and new JSON structures and returns a list of JSON paths
    that have changed.
    """
    diffs = []
    if type(old) != type(new):
        diffs.append(path)
    elif isinstance(old, dict):
        for k in set(old.keys()).union(new.keys()):
            new_path = f"{path}.{k}" if path else k
            if k not in old or k not in new:
                diffs.append(new_path)
            else:
                diffs.extend(get_diff_paths(old[k], new[k], new_path))
    elif isinstance(old, list):
        if len(old) != len(new):
            diffs.append(path)
        else:
            for i, (o, n) in enumerate(zip(old, new)):
                diffs.extend(get_diff_paths(o, n, f"{path}[{i}]"))
    else:
        if old != new:
            diffs.append(path)
    return diffs

def is_security_relevant(path):
    # path is like 'tx.events[0].topic'
    # we split by . and [ and check against SECURITY_FIELDS
    parts = path.replace('[', '.').replace(']', '').split('.')
    for part in parts:
        if part in SECURITY_FIELDS:
            return True
    return False

def main():
    parser = argparse.ArgumentParser(description="Diff snapshot JSONs and flag security-relevant changes.")
    parser.add_argument('--base', default='HEAD', help='Base commit (default: HEAD)')
    parser.add_argument('--head', default=None, help='Head commit (default: working tree)')
    args = parser.parse_args()

    files = get_changed_files(args.base, args.head)
    
    if not files:
        print("No snapshot JSON files changed.")
        sys.exit(0)

    flagged = False
    for f in files:
        old_content = get_file_content(args.base, f)
        new_content = get_file_content(args.head, f)

        try:
            old_json = json.loads(old_content) if old_content else {}
        except json.JSONDecodeError:
            old_json = {}
            
        try:
            new_json = json.loads(new_content) if new_content else {}
        except json.JSONDecodeError:
            new_json = {}

        diffs = get_diff_paths(old_json, new_json)
        
        security_diffs = [d for d in diffs if is_security_relevant(d)]
        
        if security_diffs:
            flagged = True
            print(f"\n[WARNING] Security-relevant fields changed in: {f}")
            for sd in set(security_diffs):
                print(f"  - {sd}")
        else:
            if diffs:
                print(f"\n[INFO] Changes in {f} (none are security-relevant)")

    if flagged:
        print("\nMandatory extra review required due to security-relevant snapshot changes.")
        sys.exit(1)
    else:
        print("\nNo security-relevant snapshot changes detected.")
        sys.exit(0)

if __name__ == '__main__':
    main()
