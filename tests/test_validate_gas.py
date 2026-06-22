"""
Tests for script/validate_gas.py.

These keep the docs-alignment CI coverage gate focused without invoking cargo.
"""

import importlib.util
import json
from pathlib import Path

import pytest

_SCRIPT = Path(__file__).resolve().parent.parent / "script" / "validate_gas.py"


def _load_module():
    spec = importlib.util.spec_from_file_location("validate_gas", _SCRIPT)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


vg = _load_module()


def test_extract_baselines_reads_marked_json(tmp_path):
    baseline = {"withdraw": 100, "batch_withdraw": {"10": 500}}
    path = tmp_path / "gas.md"
    path.write_text(
        "before\n"
        "<!-- GAS_BASELINE_START -->\n"
        f"{json.dumps(baseline)}\n"
        "<!-- GAS_BASELINE_END -->\n"
        "after\n",
        encoding="utf-8",
    )

    assert vg.extract_baselines(str(path)) == baseline


def test_extract_baselines_fails_without_marker(tmp_path):
    path = tmp_path / "gas.md"
    path.write_text("# no baseline here\n", encoding="utf-8")

    with pytest.raises(ValueError, match="gas baseline block"):
        vg.extract_baselines(str(path))


def test_parse_measurements_groups_by_function_and_size():
    output = "\n".join(
        [
            "noise",
            "GAS_MEASUREMENT: withdraw: single: 100",
            "GAS_MEASUREMENT: batch_withdraw: 10: 525",
            "GAS_MEASUREMENT: batch_withdraw: 50: 900",
        ]
    )

    assert vg.parse_measurements(output) == {
        "withdraw": {"single": 100},
        "batch_withdraw": {"10": 525, "50": 900},
    }


def test_run_tests_returns_stdout(monkeypatch):
    class Result:
        stdout = "GAS_MEASUREMENT: withdraw: single: 100"

    calls = []

    def fake_run(*args, **kwargs):
        calls.append((args, kwargs))
        return Result()

    monkeypatch.setattr(vg.subprocess, "run", fake_run)

    assert vg.run_tests() == Result.stdout
    assert calls
    command = calls[0][0][0]
    assert command[:4] == ["cargo", "test", "-p", "fluxora_stream"]
    assert calls[0][1]["capture_output"] is True
    assert calls[0][1]["text"] is True


def test_main_success(monkeypatch, capsys):
    monkeypatch.setattr(vg, "extract_baselines", lambda _: {"withdraw": 100})
    monkeypatch.setattr(
        vg,
        "run_tests",
        lambda: "GAS_MEASUREMENT: withdraw: single: 104\n",
    )

    with pytest.raises(SystemExit) as exc:
        vg.main()

    assert exc.value.code == 0
    assert "SUCCESS: No gas regressions detected." in capsys.readouterr().out


def test_main_detects_regression(monkeypatch, capsys):
    monkeypatch.setattr(vg, "extract_baselines", lambda _: {"withdraw": 100})
    monkeypatch.setattr(
        vg,
        "run_tests",
        lambda: "GAS_MEASUREMENT: withdraw: single: 106\n",
    )

    with pytest.raises(SystemExit) as exc:
        vg.main()

    assert exc.value.code == 1
    out = capsys.readouterr().out
    assert "FAILED: Gas regression detected" in out
    assert "withdraw (single)" in out


def test_main_handles_batch_baseline(monkeypatch, capsys):
    monkeypatch.setattr(
        vg,
        "extract_baselines",
        lambda _: {"batch_withdraw": {"10": 500}},
    )
    monkeypatch.setattr(
        vg,
        "run_tests",
        lambda: "GAS_MEASUREMENT: batch_withdraw: 10: 500\n",
    )

    with pytest.raises(SystemExit) as exc:
        vg.main()

    assert exc.value.code == 0
    assert "batch_withdraw" in capsys.readouterr().out


def test_main_reports_missing_baseline_without_failing(monkeypatch, capsys):
    monkeypatch.setattr(vg, "extract_baselines", lambda _: {})
    monkeypatch.setattr(
        vg,
        "run_tests",
        lambda: "GAS_MEASUREMENT: new_fn: single: 10\n",
    )

    with pytest.raises(SystemExit) as exc:
        vg.main()

    assert exc.value.code == 0
    assert "MISSING" in capsys.readouterr().out


def test_main_fails_when_no_measurements(monkeypatch, capsys):
    monkeypatch.setattr(vg, "extract_baselines", lambda _: {"withdraw": 100})
    monkeypatch.setattr(vg, "run_tests", lambda: "no measurements")

    with pytest.raises(SystemExit) as exc:
        vg.main()

    assert exc.value.code == 1
    assert "No gas measurements found" in capsys.readouterr().out


def test_main_wraps_exceptions(monkeypatch, capsys):
    monkeypatch.setattr(
        vg,
        "extract_baselines",
        lambda _: (_ for _ in ()).throw(RuntimeError("boom")),
    )

    with pytest.raises(SystemExit) as exc:
        vg.main()

    assert exc.value.code == 1
    assert "Error during validation: boom" in capsys.readouterr().out
