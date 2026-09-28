import json

import pytest

from script import validate_gas
from script.validate_gas import compare, entrypoints, main, parse_measurements


def test_entrypoints_match_public_abi_surface():
    names = entrypoints()
    assert len(names) == 25
    assert {"withdraw", "batch_withdraw", "delegate_withdraw"} <= names


def test_parse_measurements_reads_entrypoint_costs():
    assert parse_measurements(
        "ENTRYPOINT_COST withdraw 1050\nENTRYPOINT_COST batch_withdraw 2100"
    ) == {"withdraw": 1050, "batch_withdraw": 2100}


def test_parse_measurements_rejects_duplicate_names():
    with pytest.raises(ValueError, match="duplicate"):
        parse_measurements("ENTRYPOINT_COST withdraw 100\nENTRYPOINT_COST withdraw 200")


def test_compare_enforces_ten_percent_budget():
    rows = compare({"withdraw": 1000}, {"withdraw": 1101}, {"withdraw"})
    assert rows == [("withdraw", 1000, 1101, 1100, False)]
    assert compare({"withdraw": 1000}, {"withdraw": 1100}, {"withdraw"})[0][4]


def test_compare_rejects_incomplete_measurements():
    with pytest.raises(ValueError, match="mismatch"):
        compare({"withdraw": 100}, {}, {"withdraw"})


def test_main_reports_cost_regression(tmp_path, monkeypatch):
    baseline = tmp_path / "baseline.json"
    baseline.write_text(json.dumps({"withdraw": 1000}), encoding="utf-8")
    measurements = tmp_path / "measurements.txt"
    measurements.write_text("ENTRYPOINT_COST withdraw 1101\n", encoding="utf-8")
    report = tmp_path / "report.md"
    monkeypatch.setattr(validate_gas, "BASELINE", baseline)
    monkeypatch.setattr(validate_gas, "REPORT", report)
    monkeypatch.setattr(validate_gas, "entrypoints", lambda: {"withdraw"})

    assert main(["--measurements", str(measurements)]) == 1
    assert "| `withdraw` | 1000 | 1101 | 1100 | FAIL |" in report.read_text(encoding="utf-8")