#!/usr/bin/env python3
"""Sync the Soroban SDK version from Cargo.toml to other records."""

import re
from pathlib import Path
import sys

def main():
    repo_root = Path(__file__).resolve().parent.parent

    # 1. Read authoritative source: Cargo.toml
    cargo_toml = repo_root / "Cargo.toml"
    content = cargo_toml.read_text()
    match = re.search(r'soroban-sdk\s*=\s*"([^"]+)"', content)
    if not match:
        print("Error: Could not find soroban-sdk pin in Cargo.toml")
        sys.exit(1)
    version = match.group(1)
    major = version.split('.')[0]

    # 2. Update soroban_version.txt
    txt_file = repo_root / "soroban_version.txt"
    txt_file.write_text(version + "\n")
    print(f"Updated soroban_version.txt to {version}")

    # 3. Update rust-toolchain.toml comment
    rust_toml = repo_root / "rust-toolchain.toml"
    rust_content = rust_toml.read_text()
    # Replace `# soroban-sdk <any>.x` with `# soroban-sdk <major>.x`
    new_rust_content = re.sub(
        r'#\s*soroban-sdk\s+\d+\.x',
        f'# soroban-sdk {major}.x',
        rust_content
    )
    if new_rust_content != rust_content:
        rust_toml.write_text(new_rust_content)
        print(f"Updated rust-toolchain.toml comment to # soroban-sdk {major}.x")
    else:
        print("rust-toolchain.toml already up to date or comment not found.")

if __name__ == "__main__":
    main()
