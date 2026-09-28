import importlib.util
from pathlib import Path

import pytest


SCRIPT = Path(__file__).resolve().parents[1] / "script" / "verify_rust_version.py"
TOOLCHAIN = Path(__file__).resolve().parents[1] / "rust-toolchain.toml"


def _load_module():
    spec = importlib.util.spec_from_file_location("verify_rust_version", SCRIPT)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


verify_rust_version = _load_module()


def test_pinned_values_match_workspace_toolchain():
    assert verify_rust_version.pinned_channel(TOOLCHAIN) == "1.97.1"
    assert verify_rust_version.pinned_targets(TOOLCHAIN) == ["wasm32v1-none"]
    assert set(verify_rust_version.pinned_components(TOOLCHAIN)) == {"rustfmt", "clippy"}


def test_toolchain_loader_uses_builtin_fallback_without_tomllib(monkeypatch):
    monkeypatch.setattr(verify_rust_version, "tomllib", None)
    data = verify_rust_version._load_toolchain(TOOLCHAIN)
    assert data["toolchain"]["channel"] == "1.97.1"
    assert data["toolchain"]["targets"] == ["wasm32v1-none"]


def test_parse_rustc_version_extracts_semver():
    assert verify_rust_version.parse_rustc_version("rustc 1.97.1 (abcdef 2026-01-01)") == "1.97.1"


def test_parse_rustc_version_rejects_invalid_output():
    with pytest.raises(ValueError, match="could not parse rustc version"):
        verify_rust_version.parse_rustc_version("not rust")


def test_main_succeeds_when_pin_components_and_target_match(monkeypatch, capsys):
    monkeypatch.setenv("RUSTC_VERSION_OUTPUT", "rustc 1.97.1 (abcdef 2026-01-01)")
    monkeypatch.setenv("RUSTUP_TARGET_LIST_OUTPUT", "wasm32v1-none")
    monkeypatch.setenv("RUSTUP_COMPONENT_LIST_OUTPUT", "rustfmt\nclippy")
    assert verify_rust_version.main() == 0
    assert "Rust version matches pinned 1.97.1" in capsys.readouterr().out


def test_main_rejects_compiler_version_mismatch(monkeypatch, capsys):
    monkeypatch.setenv("RUSTC_VERSION_OUTPUT", "rustc 1.96.0 (abcdef 2026-01-01)")
    monkeypatch.setenv("RUSTUP_TARGET_LIST_OUTPUT", "wasm32v1-none")
    monkeypatch.setenv("RUSTUP_COMPONENT_LIST_OUTPUT", "rustfmt\nclippy")
    assert verify_rust_version.main() == 1
    assert "expected 1.97.1, got 1.96.0" in capsys.readouterr().err


def test_main_rejects_missing_target(monkeypatch, capsys):
    monkeypatch.setenv("RUSTC_VERSION_OUTPUT", "rustc 1.97.1 (abcdef 2026-01-01)")
    monkeypatch.setenv("RUSTUP_TARGET_LIST_OUTPUT", "x86_64-unknown-linux-gnu")
    monkeypatch.setenv("RUSTUP_COMPONENT_LIST_OUTPUT", "rustfmt\nclippy")
    assert verify_rust_version.main() == 1
    assert "Missing required targets: wasm32v1-none" in capsys.readouterr().err


def test_main_rejects_missing_component(monkeypatch, capsys):
    monkeypatch.setenv("RUSTC_VERSION_OUTPUT", "rustc 1.97.1 (abcdef 2026-01-01)")
    monkeypatch.setenv("RUSTUP_TARGET_LIST_OUTPUT", "wasm32v1-none")
    monkeypatch.setenv("RUSTUP_COMPONENT_LIST_OUTPUT", "rustfmt")
    assert verify_rust_version.main() == 1
    assert "Missing required components: clippy" in capsys.readouterr().err


def test_main_reports_invalid_toolchain_configuration(monkeypatch, capsys, tmp_path):
    toolchain = tmp_path / "rust-toolchain.toml"
    toolchain.write_text('[toolchain]\nchannel = "1.97.1"\ntargets = "invalid"\n')
    monkeypatch.setattr(verify_rust_version, "TOOLCHAIN_FILE", toolchain)
    monkeypatch.setenv("RUSTC_VERSION_OUTPUT", "rustc 1.97.1 (abcdef 2026-01-01)")
    assert verify_rust_version.main() == 1
    assert "::error::" in capsys.readouterr().err