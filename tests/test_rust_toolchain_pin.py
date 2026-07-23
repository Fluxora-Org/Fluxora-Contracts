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


# The tests above spawn a subprocess to exercise the CLI end-to-end, but code
# executed in a child process is invisible to the coverage tracer running in
# this process. The tests below call `main()`/`rustc_version()` directly
# (in-process) so `--cov=script` actually credits those lines.


def test_main_succeeds_in_process_when_rustc_matches_pin(monkeypatch, capsys):
    monkeypatch.setenv("RUSTC_VERSION_OUTPUT", "rustc 1.94.1 (abcdef 2026-01-01)")
    assert verify_rust_version.main() == 0
    captured = capsys.readouterr()
    assert "Rust version matches pinned 1.94.1" in captured.out


def test_main_fails_in_process_when_rustc_does_not_match_pin(monkeypatch, capsys):
    monkeypatch.setenv("RUSTC_VERSION_OUTPUT", "rustc 1.95.0 (abcdef 2026-02-01)")
    assert verify_rust_version.main() == 1
    captured = capsys.readouterr()
    assert "Rust version mismatch: expected 1.94.1, got 1.95.0" in captured.err


def test_main_fails_when_rustc_version_output_is_unparseable(monkeypatch, capsys):
    monkeypatch.setenv("RUSTC_VERSION_OUTPUT", "not rust at all")
    assert verify_rust_version.main() == 1
    captured = capsys.readouterr()
    assert "::error::" in captured.err


def test_rustc_version_falls_back_to_invoking_real_rustc(monkeypatch):
    # No RUSTC_VERSION_OUTPUT override: exercises the `subprocess.run(["rustc",
    # "--version"])` fallback path. Requires a real `rustc` on PATH, which is
    # guaranteed true here since this whole workspace is a Rust CI target.
    monkeypatch.delenv("RUSTC_VERSION_OUTPUT", raising=False)
    version = verify_rust_version.rustc_version()
    assert version


def test_checksum_doc_matches_pinned_channel():
    # Load pinned channel from rust-toolchain.toml using existing helper
    pinned = verify_rust_version.pinned_channel(TOOLCHAIN)
    assert pinned

    # Read checksum.rs content
    checksum_path = Path(__file__).resolve().parents[1] / "contracts" / "stream" / "src" / "checksum.rs"
    content = checksum_path.read_text(encoding="utf-8")

    # Assert the correct channel description is documented
    expected_doc = f"channel (`{pinned}`)"
    assert expected_doc in content, f"Expected checksum.rs to contain '{expected_doc}'"
