import importlib.util
import os
import subprocess
import sys
from pathlib import Path

import pytest

try:
    import tomllib
except ModuleNotFoundError:  # pragma: no cover - Python < 3.11 fallback
    import tomli as tomllib  # type: ignore[no-redef]


SCRIPT = Path(__file__).resolve().parents[1] / "script" / "verify_rust_version.py"
TOOLCHAIN = Path(__file__).resolve().parents[1] / "rust-toolchain.toml"
REPO_ROOT = Path(__file__).resolve().parents[1]
CRATE_MANIFESTS = [
    REPO_ROOT / "contracts" / "stream" / "Cargo.toml",
    REPO_ROOT / "contracts" / "factory" / "Cargo.toml",
    REPO_ROOT / "contracts" / "governance" / "Cargo.toml",
]


def _load_module():
    spec = importlib.util.spec_from_file_location("verify_rust_version", SCRIPT)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


verify_rust_version = _load_module()


def _crate_rust_version(manifest: Path) -> str:
    data = tomllib.loads(manifest.read_text(encoding="utf-8"))
    rust_version = data.get("package", {}).get("rust-version")
    if not isinstance(rust_version, str) or not rust_version:
        raise AssertionError(f"missing [package].rust-version in {manifest}")
    return rust_version


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
    env = {
        **os.environ,
        "RUSTC_VERSION_OUTPUT": "rustc 1.94.1 (abcdef 2026-01-01)",
        "RUSTUP_TARGET_LIST_OUTPUT": "wasm32-unknown-unknown",
        "RUSTUP_COMPONENT_LIST_OUTPUT": "rustfmt\nclippy",
    }
    result = subprocess.run(
        [sys.executable, str(SCRIPT)],
        capture_output=True,
        text=True,
        env=env,
    )
    assert result.returncode == 0
    assert "Rust version matches pinned 1.94.1" in result.stdout


def test_script_fails_when_rustc_does_not_match_pin():
    env = {
        **os.environ,
        "RUSTC_VERSION_OUTPUT": "rustc 1.95.0 (abcdef 2026-02-01)",
        "RUSTUP_TARGET_LIST_OUTPUT": "wasm32-unknown-unknown",
        "RUSTUP_COMPONENT_LIST_OUTPUT": "rustfmt\nclippy",
    }
    result = subprocess.run(
        [sys.executable, str(SCRIPT)],
        capture_output=True,
        text=True,
        env=env,
    )
    assert result.returncode == 1
    assert "Rust version mismatch: expected 1.94.1, got 1.95.0" in result.stderr


def test_script_fails_when_missing_targets():
    env = {
        **os.environ,
        "RUSTC_VERSION_OUTPUT": "rustc 1.94.1 (abcdef 2026-01-01)",
        "RUSTUP_TARGET_LIST_OUTPUT": "x86_64-unknown-linux-gnu",
        "RUSTUP_COMPONENT_LIST_OUTPUT": "rustfmt\nclippy",
    }
    result = subprocess.run(
        [sys.executable, str(SCRIPT)],
        capture_output=True,
        text=True,
        env=env,
    )
    assert result.returncode == 1
    assert "Missing required targets: wasm32-unknown-unknown" in result.stderr


def test_script_fails_when_missing_components():
    env = {
        **os.environ,
        "RUSTC_VERSION_OUTPUT": "rustc 1.94.1 (abcdef 2026-01-01)",
        "RUSTUP_TARGET_LIST_OUTPUT": "wasm32-unknown-unknown",
        "RUSTUP_COMPONENT_LIST_OUTPUT": "rustfmt",
    }
    result = subprocess.run(
        [sys.executable, str(SCRIPT)],
        capture_output=True,
        text=True,
        env=env,
    )
    assert result.returncode == 1
    assert "Missing required components: clippy" in result.stderr


# The tests above spawn a subprocess to exercise the CLI end-to-end, but code
# executed in a child process is invisible to the coverage tracer running in
# this process. The tests below call `main()`/`rustc_version()` directly
# (in-process) so `--cov=script` actually credits those lines.


def test_main_succeeds_in_process_when_rustc_matches_pin(monkeypatch, capsys):
    monkeypatch.setenv("RUSTC_VERSION_OUTPUT", "rustc 1.94.1 (abcdef 2026-01-01)")
    monkeypatch.setenv("RUSTUP_TARGET_LIST_OUTPUT", "wasm32-unknown-unknown")
    monkeypatch.setenv("RUSTUP_COMPONENT_LIST_OUTPUT", "rustfmt\nclippy")
    assert verify_rust_version.main() == 0
    captured = capsys.readouterr()
    assert "Rust version matches pinned 1.94.1" in captured.out


def test_main_fails_in_process_when_rustc_does_not_match_pin(monkeypatch, capsys):
    monkeypatch.setenv("RUSTC_VERSION_OUTPUT", "rustc 1.95.0 (abcdef 2026-02-01)")
    monkeypatch.setenv("RUSTUP_TARGET_LIST_OUTPUT", "wasm32-unknown-unknown")
    monkeypatch.setenv("RUSTUP_COMPONENT_LIST_OUTPUT", "rustfmt\nclippy")
    assert verify_rust_version.main() == 1
    captured = capsys.readouterr()
    assert "Rust version mismatch: expected 1.94.1, got 1.95.0" in captured.err


def test_main_fails_when_rustc_version_output_is_unparseable(monkeypatch, capsys):
    monkeypatch.setenv("RUSTC_VERSION_OUTPUT", "not rust at all")
    monkeypatch.setenv("RUSTUP_TARGET_LIST_OUTPUT", "wasm32-unknown-unknown")
    monkeypatch.setenv("RUSTUP_COMPONENT_LIST_OUTPUT", "rustfmt\nclippy")
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


def test_pinned_targets_returns_list():
    """Test pinned_targets() returns list from toolchain file."""
    targets = verify_rust_version.pinned_targets(TOOLCHAIN)
    assert isinstance(targets, list)
    assert "wasm32-unknown-unknown" in targets


def test_pinned_components_returns_list():
    """Test pinned_components() returns list from toolchain file."""
    components = verify_rust_version.pinned_components(TOOLCHAIN)
    assert isinstance(components, list)
    assert "clippy" in components
    assert "rustfmt" in components

def test_main_fails_in_process_when_missing_targets(monkeypatch, capsys):
    """Test that main() fails when required targets are missing."""
    monkeypatch.setenv("RUSTC_VERSION_OUTPUT", "rustc 1.94.1 (abcdef 2026-01-01)")
    monkeypatch.setenv("RUSTUP_TARGET_LIST_OUTPUT", "x86_64-unknown-linux-gnu")
    monkeypatch.setenv("RUSTUP_COMPONENT_LIST_OUTPUT", "rustfmt\nclippy")
    assert verify_rust_version.main() == 1
    captured = capsys.readouterr()
    assert "Missing required targets: wasm32-unknown-unknown" in captured.err


def test_main_fails_in_process_when_missing_components(monkeypatch, capsys):
    """Test that main() fails when required components are missing."""
    monkeypatch.setenv("RUSTC_VERSION_OUTPUT", "rustc 1.94.1 (abcdef 2026-01-01)")
    monkeypatch.setenv("RUSTUP_TARGET_LIST_OUTPUT", "wasm32-unknown-unknown")
    monkeypatch.setenv("RUSTUP_COMPONENT_LIST_OUTPUT", "rustfmt")
    assert verify_rust_version.main() == 1
    captured = capsys.readouterr()
    assert "Missing required components: clippy" in captured.err


def test_main_fails_when_toolchain_missing_channel(monkeypatch, capsys, tmp_path):
    """Test that main() fails when rust-toolchain.toml is missing [toolchain].channel."""
    bad_toolchain = tmp_path / "rust-toolchain.toml"
    bad_toolchain.write_text('[toolchain]\n')
    
    monkeypatch.setattr(verify_rust_version, "TOOLCHAIN_FILE", bad_toolchain)
    assert verify_rust_version.main() == 1
    captured = capsys.readouterr()
    assert "::error::" in captured.err


def test_pinned_targets_raises_on_invalid_type(tmp_path):
    """Test that pinned_targets() raises ValueError when targets is not a list."""
    bad_toolchain = tmp_path / "rust-toolchain.toml"
    bad_toolchain.write_text('[toolchain]\nchannel = "1.94.1"\ntargets = "not-a-list"\n')
    
    with pytest.raises(ValueError, match="invalid.*targets"):
        verify_rust_version.pinned_targets(bad_toolchain)


def test_pinned_components_raises_on_invalid_type(tmp_path):
    """Test that pinned_components() raises ValueError when components is not a list."""
    bad_toolchain = tmp_path / "rust-toolchain.toml"
    bad_toolchain.write_text('[toolchain]\nchannel = "1.94.1"\ncomponents = "not-a-list"\n')
    
    with pytest.raises(ValueError, match="invalid.*components"):
        verify_rust_version.pinned_components(bad_toolchain)


def test_main_fails_when_exception_during_loading(monkeypatch, capsys, tmp_path):
    """Test that main() exits with error code 1 when exception occurs during loading."""
    bad_toolchain = tmp_path / "rust-toolchain.toml"
    bad_toolchain.write_text('[toolchain]\nchannel = "1.94.1"\ntargets = "invalid"\n')
    
    monkeypatch.setattr(verify_rust_version, "TOOLCHAIN_FILE", bad_toolchain)
    monkeypatch.setenv("RUSTC_VERSION_OUTPUT", "rustc 1.94.1 (abcdef 2026-01-01)")
    monkeypatch.setenv("RUSTUP_TARGET_LIST_OUTPUT", "wasm32-unknown-unknown")
    monkeypatch.setenv("RUSTUP_COMPONENT_LIST_OUTPUT", "rustfmt\nclippy")
    
    result = verify_rust_version.main()
    assert result == 1
    captured = capsys.readouterr()
    assert "::error::" in captured.err

def test_main_prints_installed_targets_message(monkeypatch, capsys):
    """Test that main() prints targets match message when all present."""
    monkeypatch.setenv("RUSTC_VERSION_OUTPUT", "rustc 1.94.1 (abcdef 2026-01-01)")
    monkeypatch.setenv("RUSTUP_TARGET_LIST_OUTPUT", "wasm32-unknown-unknown")
    monkeypatch.setenv("RUSTUP_COMPONENT_LIST_OUTPUT", "rustfmt\nclippy")
    assert verify_rust_version.main() == 0
    captured = capsys.readouterr()
    assert "Installed targets match requirements" in captured.out


def test_main_prints_installed_components_message(monkeypatch, capsys):
    """Test that main() prints components match message when all present."""
    monkeypatch.setenv("RUSTC_VERSION_OUTPUT", "rustc 1.94.1 (abcdef 2026-01-01)")
    monkeypatch.setenv("RUSTUP_TARGET_LIST_OUTPUT", "wasm32-unknown-unknown")
    monkeypatch.setenv("RUSTUP_COMPONENT_LIST_OUTPUT", "rustfmt\nclippy")
    assert verify_rust_version.main() == 0
    captured = capsys.readouterr()
    assert "Installed components match requirements" in captured.out

# MSRV cross-check: each crate manifest's `rust-version` must track the
# `rust-toolchain.toml` pin independently of the CI-invoked `rustc --version`
# comparison above, so `cargo` itself enforces the floor on every invocation
# (including local developer builds), not just CI.


@pytest.mark.parametrize(
    "manifest", CRATE_MANIFESTS, ids=lambda p: p.relative_to(REPO_ROOT).as_posix()
)
def test_crate_rust_version_matches_pinned_toolchain(manifest):
    assert _crate_rust_version(manifest) == verify_rust_version.pinned_channel(TOOLCHAIN)


def test_parse_toml_simple_fallback():
    content = '[toolchain]\nchannel = "1.94.1"\ncomponents = ["rustfmt", "clippy"]\ntargets = ["wasm32-unknown-unknown"]\n'
    parsed = verify_rust_version._parse_toml_simple(content)
    assert parsed["toolchain"]["channel"] == "1.94.1"
    assert parsed["toolchain"]["components"] == ["rustfmt", "clippy"]
    assert parsed["toolchain"]["targets"] == ["wasm32-unknown-unknown"]


def test_load_toolchain_fallback_when_tomllib_none(monkeypatch):
    monkeypatch.setattr(verify_rust_version, "tomllib", None)
    data = verify_rust_version._load_toolchain(TOOLCHAIN)
    assert data["toolchain"]["channel"] == "1.94.1"


