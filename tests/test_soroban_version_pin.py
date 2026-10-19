import importlib.util
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
SCRIPT = REPO_ROOT / "script" / "verify_soroban_version.py"

spec = importlib.util.spec_from_file_location("verify_soroban_version", SCRIPT)
verify_soroban_version = importlib.util.module_from_spec(spec)
spec.loader.exec_module(verify_soroban_version)


def _write_records(root, cargo_version="27.0.5", text_version="27.0.5", toolchain_major="27"):
    (root / "Cargo.toml").write_text(
        f'[workspace.dependencies]\nsoroban-sdk = "{cargo_version}"\n', encoding="utf-8"
    )
    (root / "soroban_version.txt").write_text(f"{text_version}\n", encoding="utf-8")
    (root / "rust-toolchain.toml").write_text(
        f"# soroban-sdk {toolchain_major}.x\n[toolchain]\nchannel = \"1.97.1\"\n",
        encoding="utf-8",
    )


def test_soroban_versions_agree():
    assert verify_soroban_version.check_versions(REPO_ROOT) == []


def test_soroban_version_mismatches_are_reported(tmp_path):
    _write_records(tmp_path, text_version="26.0.0", toolchain_major="26")

    issues = verify_soroban_version.check_versions(tmp_path)

    assert len(issues) == 2
    assert "soroban_version.txt" in issues[0]
    assert "rust-toolchain.toml" in issues[1]
