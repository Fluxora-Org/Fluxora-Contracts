#!/usr/bin/env python3
"""Sync Soroban SDK version mirrors from Cargo.toml's workspace pin."""

import re
from pathlib import Path
import sys

def main():
    repo_root = Path(__file__).resolve().parent.parent

    # Cargo.toml [workspace.dependencies].soroban-sdk is authoritative.
    cargo_toml = repo_root / "Cargo.toml"
    content = cargo_toml.read_text()
    workspace_dependencies = re.search(
        r"(?ms)^\[workspace\.dependencies\]\s*(.*?)(?=^\[|\Z)", content
    )
    matches = (
        re.findall(
            r'^\s*soroban-sdk\s*=\s*"([^"]+)"\s*$',
            workspace_dependencies.group(1),
            re.M,
        )
        if workspace_dependencies
        else []
    )
    if len(matches) != 1:
        print("Error: Could not find exactly one workspace soroban-sdk pin in Cargo.toml")
        sys.exit(1)
    version = matches[0]
    major = version.split('.')[0]

    # 2. Prepare soroban_version.txt update
    txt_file = repo_root / "soroban_version.txt"

    # 3. Prepare rust-toolchain.toml comment update
    rust_toml = repo_root / "rust-toolchain.toml"
    rust_content = rust_toml.read_text()
    # Replace `# soroban-sdk <any>.x` with `# soroban-sdk <major>.x`
    new_rust_content, replacements = re.subn(
        r'#\s*soroban-sdk\s+\d+\.x',
        f'# soroban-sdk {major}.x',
        rust_content
    )
    if replacements != 1:
        print("Error: expected exactly one '# soroban-sdk <X>.x' comment in rust-toolchain.toml")
        sys.exit(1)

    txt_file.write_text(version + "\n")
    rust_toml.write_text(new_rust_content)
    print(f"Updated soroban_version.txt and rust-toolchain.toml for Soroban SDK {version}")

if __name__ == "__main__":
    main()
