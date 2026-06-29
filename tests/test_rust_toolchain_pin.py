import importlib.util
import os
import subprocess
import sys
from pathlib import Path


SCRIPT = Path(__file__).resolve().parents[1] / "script" / "verify_rust_version.py"
TOOLCHAIN = Path(__file__).resolve().parents[1] / "rust-toolchain.toml"


def _load_module():
    spec = importlib.util.spec_from_file_location("verify_rust_version", SCRIPT)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


verify_rust_version = _load_module()


def test_pinned_channel_reads_rust_toolchain_toml():
    assert verify_rust_version.pinned_channel(TOOLCHAIN) == "1.94.1"


def test_parse_rustc_version_extracts_semver():
    assert (
        verify_rust_version.parse_rustc_version("rustc 1.94.1 (abcdef 2026-01-01)")
        == "1.94.1"
    )


def test_parse_rustc_version_rejects_unexpected_output():
    try:
        verify_rust_version.parse_rustc_version("not rust")
    except ValueError as exc:
        assert "could not parse rustc version" in str(exc)
    else:
        raise AssertionError("expected ValueError")


def test_script_succeeds_when_rustc_matches_pin():
    env = {**os.environ, "RUSTC_VERSION_OUTPUT": "rustc 1.94.1 (abcdef 2026-01-01)"}
    result = subprocess.run(
        [sys.executable, str(SCRIPT)],
        capture_output=True,
        text=True,
        env=env,
    )
    assert result.returncode == 0
    assert "Rust version matches pinned 1.94.1" in result.stdout


def test_script_fails_when_rustc_does_not_match_pin():
    env = {**os.environ, "RUSTC_VERSION_OUTPUT": "rustc 1.95.0 (abcdef 2026-02-01)"}
    result = subprocess.run(
        [sys.executable, str(SCRIPT)],
        capture_output=True,
        text=True,
        env=env,
    )
    assert result.returncode == 1
    assert "Rust version mismatch: expected 1.94.1, got 1.95.0" in result.stderr
