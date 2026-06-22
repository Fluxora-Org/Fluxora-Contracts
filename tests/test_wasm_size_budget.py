"""
Tests for script/check-wasm-size-budget.sh.

The functional checks run only when bash is available. GitHub Actions Ubuntu
runners execute them; Windows development machines without bash still get the
static coverage.
"""

from __future__ import annotations

import os
import platform
import shutil
import subprocess
from pathlib import Path

import pytest


REPO_ROOT = Path(__file__).resolve().parent.parent
SCRIPT = REPO_ROOT / "script" / "check-wasm-size-budget.sh"


def test_budget_script_covers_all_workspace_contracts():
    text = SCRIPT.read_text(encoding="utf-8")

    for contract in ("fluxora_stream", "fluxora_factory", "fluxora_governance"):
        assert contract in text
        assert f"-p {contract}" in text


def test_budget_script_exposes_per_contract_budget_overrides():
    text = SCRIPT.read_text(encoding="utf-8")

    assert "FLUXORA_STREAM_WASM_BUDGET_BYTES" in text
    assert "FLUXORA_FACTORY_WASM_BUDGET_BYTES" in text
    assert "FLUXORA_GOVERNANCE_WASM_BUDGET_BYTES" in text


def test_budget_script_reports_missing_required_raw_artifacts():
    text = SCRIPT.read_text(encoding="utf-8")

    assert "MISSING ${contract} ${kind} artifact" in text
    assert "optimized | not present" in text


def _run_budget_script(release_dir: Path, report_file: Path, env: dict[str, str]):
    bash = shutil.which("bash")
    if bash is None:
        pytest.skip("bash is not available on this machine")
    if platform.system() == "Windows" and "WindowsApps" in bash:
        pytest.skip("WSL bash cannot consume raw Windows paths in this test")

    return subprocess.run(
        [
            bash,
            str(SCRIPT),
            "--no-build",
            "--release-dir",
            str(release_dir),
            "--report-file",
            str(report_file),
        ],
        cwd=REPO_ROOT,
        env={**os.environ, **env},
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )


def _write_wasm_artifacts(release_dir: Path, stream_size: int = 10):
    release_dir.mkdir(parents=True)
    sizes = {
        "fluxora_stream.wasm": stream_size,
        "fluxora_factory.wasm": 8,
        "fluxora_governance.wasm": 6,
    }
    for name, size in sizes.items():
        (release_dir / name).write_bytes(b"x" * size)


def test_budget_script_passes_when_artifacts_fit_budget(tmp_path: Path):
    release_dir = tmp_path / "release"
    report = tmp_path / "report.md"
    _write_wasm_artifacts(release_dir)

    result = _run_budget_script(
        release_dir,
        report,
        {
            "FLUXORA_STREAM_WASM_BUDGET_BYTES": "20",
            "FLUXORA_FACTORY_WASM_BUDGET_BYTES": "20",
            "FLUXORA_GOVERNANCE_WASM_BUDGET_BYTES": "20",
        },
    )

    assert result.returncode == 0, result.stderr
    report_text = report.read_text(encoding="utf-8")
    assert "| `fluxora_stream` | raw | 10 | 20 | PASS |" in report_text
    assert "| `fluxora_factory` | optimized | not present | 20 | SKIP |" in report_text


def test_budget_script_fails_when_any_artifact_exceeds_budget(tmp_path: Path):
    release_dir = tmp_path / "release"
    report = tmp_path / "report.md"
    _write_wasm_artifacts(release_dir, stream_size=12)

    result = _run_budget_script(
        release_dir,
        report,
        {
            "FLUXORA_STREAM_WASM_BUDGET_BYTES": "10",
            "FLUXORA_FACTORY_WASM_BUDGET_BYTES": "20",
            "FLUXORA_GOVERNANCE_WASM_BUDGET_BYTES": "20",
        },
    )

    assert result.returncode == 1
    assert "FAIL fluxora_stream raw: 12 bytes > 10" in result.stdout
    assert "| `fluxora_stream` | raw | 12 | 10 | FAIL |" in report.read_text(
        encoding="utf-8"
    )
