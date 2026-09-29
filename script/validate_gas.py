#!/usr/bin/env python3
"""Measure all stream ABI calls and compare CPU instructions to a pinned baseline."""

import argparse
import json
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BASELINE = ROOT / "contracts/stream/entrypoint-cost-baseline.json"
REPORT = ROOT / "target/entrypoint-cost-report.md"
LIB = ROOT / "contracts/stream/src/lib.rs"
TOLERANCE_PERCENT = 10


def entrypoints():
    return set(re.findall(r"^    pub fn (\w+)\(", LIB.read_text(encoding="utf-8"), re.M))


def parse_measurements(output):
    measurements = {}
    for name, value in re.findall(r"\bENTRYPOINT_COST (\w+) (\d+)\b", output):
        if name in measurements:
            raise ValueError(f"duplicate measurement: {name}")
        measurements[name] = int(value)
    return measurements


def run_tests():
    build = subprocess.run(
        [
            "cargo", "build", "--locked", "--release", "-p", "fluxora-stream",
            "--target", "wasm32v1-none",
        ],
        cwd=ROOT,
        text=True,
        capture_output=True,
    )
    if build.returncode:
        raise RuntimeError(
            f"release WASM build failed ({build.returncode}):\n"
            f"{build.stdout}\n{build.stderr}"
        )
    command = [
        "cargo", "test", "--locked", "-p", "fluxora-stream", "--lib",
        "entrypoint_cost_snapshot", "--", "--ignored", "--nocapture",
        "--test-threads=1",
    ]
    result = subprocess.run(command, cwd=ROOT, text=True, capture_output=True)
    if result.returncode:
        raise RuntimeError(
            f"cost fixture failed ({result.returncode}):\n{result.stdout}\n{result.stderr}"
        )
    return result.stdout + "\n" + result.stderr


def compare(baseline, measured, expected):
    if set(baseline) != expected or set(measured) != expected:
        raise ValueError(
            f"ABI/measurement mismatch: expected={sorted(expected)}, "
            f"baseline={sorted(baseline)}, measured={sorted(measured)}"
        )
    rows = []
    for name in sorted(expected):
        old = baseline[name]
        new = measured[name]
        if type(old) is not int or old <= 0 or new <= 0:
            raise ValueError(f"invalid instruction count for {name}")
        limit = old + old * TOLERANCE_PERCENT // 100
        rows.append((name, old, new, limit, new <= limit))
    return rows


def write_report(rows, error=None):
    REPORT.parent.mkdir(parents=True, exist_ok=True)
    lines = ["# Stream entry point CPU cost", "", f"Tolerance: +{TOLERANCE_PERCENT}%", ""]
    if error:
        lines.extend([f"Measurement failed: {error}", ""])
    else:
        lines.extend(
            [
                "Costs are Soroban SDK instruction estimates for successful calls against",
                "the release WASM in `entrypoint_costs.rs`; setup calls are excluded.",
                "These estimates are local simulation measurements, not network fees.",
                "",
                "| Entry point | Baseline | Measured | Maximum | Result |",
                "| --- | ---: | ---: | ---: | --- |",
            ]
        )
        for name, old, new, limit, passed in rows:
            result = "PASS" if passed else "FAIL"
            lines.append(f"| `{name}` | {old} | {new} | {limit} | {result} |")
        lines.append("")
    REPORT.write_text("\n".join(lines), encoding="utf-8")


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--record-baseline", action="store_true",
        help="replace baseline from a successful full measurement",
    )
    parser.add_argument(
        "--measurements", type=Path,
        help="read saved cargo test output instead of running cargo",
    )
    args = parser.parse_args(argv)
    try:
        output = (
            args.measurements.read_text(encoding="utf-8")
            if args.measurements else run_tests()
        )
        measured = parse_measurements(output)
        expected = entrypoints()
        if set(measured) != expected:
            raise ValueError(
                f"incomplete measurements: missing={sorted(expected - set(measured))}, "
                f"extra={sorted(set(measured) - expected)}"
            )
        if args.record_baseline:
            BASELINE.write_text(
                json.dumps(dict(sorted(measured.items())), indent=2) + "\n",
                encoding="utf-8",
            )
        baseline = json.loads(BASELINE.read_text(encoding="utf-8"))
        rows = compare(baseline, measured, expected)
        write_report(rows)
        print(REPORT.read_text(encoding="utf-8"))
        return 0 if all(row[4] for row in rows) else 1
    except (OSError, ValueError, RuntimeError) as exc:
        write_report([], str(exc))
        print(f"Cost gate failed: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
