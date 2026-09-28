import re
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent

def test_soroban_versions_agree():
    # 1. Cargo.toml (Authoritative)
    cargo_toml = REPO_ROOT / "Cargo.toml"
    cargo_content = cargo_toml.read_text()
    cargo_match = re.search(r'soroban-sdk\s*=\s*"([^"]+)"', cargo_content)
    assert cargo_match is not None, "Could not find soroban-sdk pin in Cargo.toml"
    cargo_version = cargo_match.group(1)

    # 2. soroban_version.txt
    soroban_txt = REPO_ROOT / "soroban_version.txt"
    txt_version = soroban_txt.read_text().strip()

    # 3. rust-toolchain.toml (comment)
    rust_toml = REPO_ROOT / "rust-toolchain.toml"
    rust_content = rust_toml.read_text()
    rust_match = re.search(r'#\s*soroban-sdk\s+(\d+)\.x', rust_content)
    assert rust_match is not None, "Could not find '# soroban-sdk <X>.x' comment in rust-toolchain.toml"
    rust_major = rust_match.group(1)

    # Validate major version
    cargo_major = cargo_version.split('.')[0]
    
    error_msg = (
        f"Mismatch detected!\n"
        f"  Cargo.toml (authoritative): {cargo_version}\n"
        f"  soroban_version.txt: {txt_version}\n"
        f"  rust-toolchain.toml: {rust_major}.x\n\n"
        "The authoritative source is Cargo.toml.\n"
        "Updating the target version is a single documented edit:\n"
        "1. Update `soroban-sdk` in Cargo.toml\n"
        "2. Run `python script/update_soroban_version.py` to sync all records."
    )

    assert txt_version == cargo_version, error_msg
    assert rust_major == cargo_major, error_msg
