import importlib.util
from pathlib import Path
import sys

import pytest


SCRIPT = Path(__file__).parents[1] / "script" / "check_stream_coverage.py"
SPEC = importlib.util.spec_from_file_location("check_stream_coverage", SCRIPT)
coverage = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
sys.modules[SPEC.name] = coverage
SPEC.loader.exec_module(coverage)


REPORT = """\
<coverage line-rate="0.964" version="1">
  <packages>
    <package name="stream">
      <classes>
        <class filename="src/lib.rs" line-rate="0.970" />
        <class filename="src/accrual.rs" line-rate="0.900" />
      </classes>
    </package>
  </packages>
</coverage>
"""


def write_report(tmp_path, content=REPORT):
    report = tmp_path / "cobertura.xml"
    report.write_text(content, encoding="utf-8")
    return report


def test_read_coverage_returns_aggregate_and_sorted_modules(tmp_path):
    actual, modules = coverage.read_coverage(write_report(tmp_path))

    assert actual == coverage.Decimal("96.4")
    assert [(module.filename, module.percentage) for module in modules] == [
        ("src/accrual.rs", coverage.Decimal("90.0")),
        ("src/lib.rs", coverage.Decimal("97.0")),
    ]


def test_main_passes_at_committed_floor_and_writes_module_summary(tmp_path):
    report = write_report(tmp_path)
    floor = tmp_path / "coverage-floor.txt"
    floor.write_text("96.4\n", encoding="utf-8")
    summary = tmp_path / "summary.md"
    output = tmp_path / "github-output"

    assert coverage.main(
        [
            "--xml",
            str(report),
            "--floor",
            str(floor),
            "--summary",
            str(summary),
            "--github-output",
            str(output),
        ]
    ) == 0
    assert "`src/accrual.rs`" in summary.read_text(encoding="utf-8")
    assert "coverage_pct=96.4" in output.read_text(encoding="utf-8")


def test_main_fails_below_floor(tmp_path):
    report = write_report(tmp_path)
    floor = tmp_path / "coverage-floor.txt"
    floor.write_text("96.5\n", encoding="utf-8")

    assert coverage.main(["--xml", str(report), "--floor", str(floor)]) == 1


@pytest.mark.parametrize("value", ["", "101", "not-a-number"])
def test_read_floor_rejects_invalid_values(tmp_path, value):
    floor = tmp_path / "coverage-floor.txt"
    floor.write_text(value, encoding="utf-8")

    with pytest.raises(ValueError):
        coverage.read_floor(floor)