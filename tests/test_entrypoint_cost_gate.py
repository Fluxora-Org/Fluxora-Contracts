import pytest

from script import validate_gas
from script.validate_gas import compare, entrypoints, parse_measurements


def test_inventory_is_all_24_abi_entries():
    names = entrypoints()
    assert len(names) == 24
    assert {"withdraw", "batch_withdraw", "delegate_withdraw"} <= names


def test_parse_rejects_duplicate_measurement():
    with pytest.raises(ValueError, match="duplicate"):
        parse_measurements("ENTRYPOINT_COST withdraw 100\nENTRYPOINT_COST withdraw 200")


def test_compare_rejects_missing_entry():
    with pytest.raises(ValueError, match="mismatch"):
        compare({"withdraw": 100}, {}, {"withdraw"})


def test_withdraw_regression_fails_beyond_ten_percent():
    rows = compare({"withdraw": 1000}, {"withdraw": 1101}, {"withdraw"})
    assert rows == [("withdraw", 1000, 1101, 1100, False)]
    assert compare({"withdraw": 1000}, {"withdraw": 1100}, {"withdraw"})[0][4]


def test_gate_fails_and_publishes_report_for_withdraw_regression(tmp_path, monkeypatch):
    baseline = tmp_path / "baseline.json"
    baseline.write_text('{"withdraw": 1000}', encoding="utf-8")
    measurements = tmp_path / "measurements.txt"
    measurements.write_text("ENTRYPOINT_COST withdraw 1101\n", encoding="utf-8")
    report = tmp_path / "report.md"
    monkeypatch.setattr(validate_gas, "BASELINE", baseline)
    monkeypatch.setattr(validate_gas, "REPORT", report)
    monkeypatch.setattr(validate_gas, "entrypoints", lambda: {"withdraw"})

    assert validate_gas.main(["--measurements", str(measurements)]) == 1
    assert "| `withdraw` | 1000 | 1101 | 1100 | FAIL |" in report.read_text(encoding="utf-8")
