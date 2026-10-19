#!/usr/bin/env python3
"""Enforce and summarize a committed line-coverage floor for every workspace crate.

Coverage used to be gated for ``fluxora-stream`` alone. CI measured that one
crate, compared it against ``contracts/stream/coverage-floor.txt``, and every
other workspace member -- the factory, the archival probe, the provenance tool
-- could lose its tests without anything noticing. A crate added later would
arrive with no floor at all, so the gate would not even know to look.

This script is the workspace-wide replacement. It derives the crate list from
``cargo metadata``, which is the same source cargo itself uses to resolve
``[workspace] members``, so a crate is in scope the moment it is added. For
every crate it then:

* requires a committed ``coverage-floor.txt`` next to that crate's
  ``Cargo.toml`` -- a crate added without one fails the gate;
* requires a Cobertura report for that crate, so a crate that was silently
  dropped from the measurement step cannot pass by being absent;
* fails when line coverage falls below the committed floor.

Every crate's measured figure, committed floor and status are written to the
job summary, so a reviewer reads the whole workspace out of one table rather
than one number per run.

The floor is a ratchet
---------------------

A floor may be raised once the new baseline has been measured, and only by
committing the measured value. Lowering a floor is never a valid way to
resolve a coverage failure: it deletes the evidence that tests were removed.
The floor files are routed to the maintainers team in ``.github/CODEOWNERS``
so a lowering cannot land unreviewed.

Measuring a crate
-----------------

``script/measure-coverage.sh`` produces one report per crate under
``coverage/<crate-name>/cobertura.xml`` and is the supported way to run the
measurement. To reproduce the reports and the gate locally::

    cargo install cargo-tarpaulin --version 0.31 --locked
    script/measure-coverage.sh
    python3 script/check_coverage_floors.py \\
        --reports-dir coverage \\
        --summary coverage/summary.md

Exit codes
----------

* ``0`` -- every crate has a floor, a report, and meets it.
* ``1`` -- at least one crate is below its floor, has no committed floor, has
  no report, or has an unreadable floor/report.
* ``2`` -- broken input: the workspace metadata could not be read at all.
"""

from __future__ import annotations

import argparse
import json
import subprocess
import sys
import xml.etree.ElementTree as ET
from dataclasses import dataclass
from decimal import Decimal, InvalidOperation
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent

#: The committed floor, one file per crate, next to that crate's ``Cargo.toml``.
FLOOR_FILENAME = "coverage-floor.txt"

#: The Cobertura report ``script/measure-coverage.sh`` writes per crate.
REPORT_FILENAME = "cobertura.xml"

#: Package name -> git-ignored or explicit feature the measurement must enable.
#: Only crates that actually declare ``testutils`` get the flag; passing it to a
#: crate that does not declare it is a hard cargo error, which would otherwise
#: make a new crate unmeasurable until this table was edited by hand.
DEFAULT_MEASUREMENT_FEATURE = "testutils"

STATUS_PASSED = "passed"
STATUS_BELOW_FLOOR = "below-floor"
STATUS_MISSING_FLOOR = "missing-floor"
STATUS_MISSING_REPORT = "missing-report"
STATUS_UNREADABLE_FLOOR = "unreadable-floor"
STATUS_UNREADABLE_REPORT = "unreadable-report"

_FAILING_STATUSES = frozenset(
    {
        STATUS_BELOW_FLOOR,
        STATUS_MISSING_FLOOR,
        STATUS_MISSING_REPORT,
        STATUS_UNREADABLE_FLOOR,
        STATUS_UNREADABLE_REPORT,
    }
)


@dataclass(frozen=True)
class ModuleCoverage:
    filename: str
    percentage: Decimal


@dataclass(frozen=True)
class Crate:
    """A workspace member, as cargo resolved it."""

    name: str
    #: Crate directory relative to the repository root.
    directory: str

    def floor_path_under(self, repo_root: Path) -> Path:
        return repo_root / self.directory / FLOOR_FILENAME

    def report_path_under(self, reports_dir: Path) -> Path:
        return reports_dir / self.name / REPORT_FILENAME


@dataclass(frozen=True)
class CrateResult:
    crate: Crate
    floor: Decimal | None
    actual: Decimal | None
    modules: tuple[ModuleCoverage, ...]
    status: str
    detail: str

    @property
    def passed(self) -> bool:
        return self.status == STATUS_PASSED


class BrokenWorkspaceError(Exception):
    """The workspace metadata could not be read, so no crate is in scope."""


def _percentage(rate: str) -> Decimal:
    try:
        return Decimal(rate) * Decimal("100")
    except InvalidOperation as error:
        raise ValueError(f"invalid coverage rate: {rate!r}") from error


def format_percentage(value: Decimal) -> str:
    return f"{value:.2f}".rstrip("0").rstrip(".")


def read_coverage(xml_path: Path) -> tuple[Decimal, list[ModuleCoverage]]:
    """Return the aggregate line coverage and the per-module breakdown."""
    root = ET.parse(xml_path).getroot()
    line_rate = root.attrib.get("line-rate")
    if line_rate is None:
        raise ValueError("coverage report has no root line-rate")

    modules = []
    for class_element in root.findall(".//class"):
        filename = class_element.attrib.get("filename")
        module_rate = class_element.attrib.get("line-rate")
        if filename is None or module_rate is None:
            continue
        modules.append(ModuleCoverage(filename, _percentage(module_rate)))

    return _percentage(line_rate), sorted(modules, key=lambda module: module.filename)


def read_floor(floor_path: Path) -> Decimal:
    value = floor_path.read_text(encoding="utf-8").strip()
    try:
        floor = Decimal(value)
    except InvalidOperation as error:
        raise ValueError(f"invalid coverage floor: {value!r}") from error
    if floor < 0 or floor > 100:
        raise ValueError(f"coverage floor must be between 0 and 100: {floor}")
    return floor


def read_metadata(repo_root: Path = REPO_ROOT) -> dict:
    """Run ``cargo metadata`` and return the parsed document.

    ``cargo metadata`` is used rather than a hand-rolled parse of the root
    ``Cargo.toml`` so that the gate's notion of "a crate" is exactly cargo's:
    a new ``[workspace] members`` entry is in scope without this script
    needing to know anything about it.
    """
    try:
        completed = subprocess.run(
            ["cargo", "metadata", "--format-version", "1", "--no-deps"],
            cwd=repo_root,
            check=True,
            capture_output=True,
            text=True,
        )
    except (OSError, subprocess.CalledProcessError) as error:
        detail = getattr(error, "stderr", None) or str(error)
        raise BrokenWorkspaceError(f"cargo metadata failed: {detail.strip()}") from error

    try:
        return json.loads(completed.stdout)
    except json.JSONDecodeError as error:
        raise BrokenWorkspaceError(f"cargo metadata is not valid JSON: {error}") from error


def workspace_crates(
    metadata: dict, repo_root: Path = REPO_ROOT
) -> list[Crate]:
    """Every workspace member cargo resolved, sorted by package name."""
    packages = metadata.get("packages") or []
    if not packages:
        raise BrokenWorkspaceError("cargo metadata reports no packages")

    crates = []
    for package in packages:
        name = package.get("name")
        manifest_path = package.get("manifest_path")
        if not name or not manifest_path:
            raise BrokenWorkspaceError(
                "a package in cargo metadata has no name or manifest_path"
            )
        directory = Path(manifest_path).parent
        try:
            relative = directory.resolve().relative_to(repo_root.resolve())
        except ValueError as error:
            raise BrokenWorkspaceError(
                f"package {name} is outside the repository: {directory}"
            ) from error
        crates.append(Crate(name=name, directory=relative.as_posix()))

    return sorted(crates, key=lambda crate: crate.name)


def measurement_feature(crate: Crate, metadata: dict) -> str | None:
    """The feature the measurement must enable for this crate, if any.

    Returns ``None`` for a crate that declares no ``testutils`` feature, which
    is what lets a newly added crate be measured without editing this script.
    """
    for package in metadata.get("packages") or []:
        if package.get("name") != crate.name:
            continue
        features = package.get("features") or {}
        if DEFAULT_MEASUREMENT_FEATURE in features:
            return DEFAULT_MEASUREMENT_FEATURE
    return None


def evaluate_crate(
    crate: Crate, reports_dir: Path, repo_root: Path = REPO_ROOT
) -> CrateResult:
    """Measure one crate against its committed floor."""
    floor_path = crate.floor_path_under(repo_root)
    if not floor_path.is_file():
        return CrateResult(
            crate=crate,
            floor=None,
            actual=None,
            modules=(),
            status=STATUS_MISSING_FLOOR,
            detail=(
                f"no committed floor at {crate.directory}/{FLOOR_FILENAME}. "
                "Measure the crate, then commit its measured line coverage as "
                "the floor."
            ),
        )

    try:
        floor = read_floor(floor_path)
    except (OSError, ValueError) as error:
        return CrateResult(
            crate=crate,
            floor=None,
            actual=None,
            modules=(),
            status=STATUS_UNREADABLE_FLOOR,
            detail=f"{crate.directory}/{FLOOR_FILENAME}: {error}",
        )

    report = crate.report_path_under(reports_dir)
    if not report.is_file():
        return CrateResult(
            crate=crate,
            floor=floor,
            actual=None,
            modules=(),
            status=STATUS_MISSING_REPORT,
            detail=(
                f"no coverage report at {report}. Run script/measure-coverage.sh "
                "so every workspace crate is measured."
            ),
        )

    try:
        actual, modules = read_coverage(report)
    except (OSError, ET.ParseError, ValueError) as error:
        return CrateResult(
            crate=crate,
            floor=floor,
            actual=None,
            modules=(),
            status=STATUS_UNREADABLE_REPORT,
            detail=f"{report}: {error}",
        )

    if actual < floor:
        return CrateResult(
            crate=crate,
            floor=floor,
            actual=actual,
            modules=tuple(modules),
            status=STATUS_BELOW_FLOOR,
            detail=(
                f"line coverage {format_percentage(actual)}% is below the "
                f"committed floor {format_percentage(floor)}%."
            ),
        )

    return CrateResult(
        crate=crate,
        floor=floor,
        actual=actual,
        modules=tuple(modules),
        status=STATUS_PASSED,
        detail="",
    )


def evaluate(
    crates: list[Crate], reports_dir: Path, repo_root: Path = REPO_ROOT
) -> list[CrateResult]:
    return [evaluate_crate(crate, reports_dir, repo_root) for crate in crates]


def write_summary(summary_path: Path, results: list[CrateResult]) -> None:
    """Write the whole workspace's coverage to the job summary."""
    lines = [
        "## Workspace Coverage",
        "",
        "One committed floor per workspace crate, enforced by "
        "`script/check_coverage_floors.py`.",
        "",
        "| Crate | Path | Line coverage | Committed floor | Status |",
        "|------|------|---------------|-----------------|--------|",
    ]

    for result in results:
        actual = (
            f"{format_percentage(result.actual)}%"
            if result.actual is not None
            else "not measured"
        )
        floor = (
            f"{format_percentage(result.floor)}%"
            if result.floor is not None
            else "none committed"
        )
        lines.append(
            f"| `{result.crate.name}` | `{result.crate.directory}` | {actual} | "
            f"{floor} | {result.status} |"
        )

    failing = [result for result in results if not result.passed]
    if failing:
        lines.extend(["", "### Gate failures", ""])
        lines.extend(f"* `{result.crate.name}`: {result.detail}" for result in failing)

    for result in results:
        if not result.modules:
            continue
        lines.extend(
            [
                "",
                f"### `{result.crate.name}` -- coverage by module",
                "",
                "| Module | Line coverage |",
                "|--------|---------------|",
            ]
        )
        lines.extend(
            f"| `{module.filename}` | {format_percentage(module.percentage)}% |"
            for module in result.modules
        )

    with summary_path.open("a", encoding="utf-8") as summary:
        summary.write("\n".join(lines) + "\n")


def write_github_output(output_path: Path, results: list[CrateResult]) -> None:
    """Expose each crate's measured figure as a step output.

    Keys are ``coverage_<package name>``, so a downstream step can gate on one
    crate without re-parsing the report.
    """
    with output_path.open("a", encoding="utf-8") as output:
        for result in results:
            if result.actual is None:
                continue
            output.write(
                f"coverage_{result.crate.name}="
                f"{format_percentage(result.actual)}\n"
            )


def list_crates_line(crate: Crate, feature: str | None) -> str:
    return f"{crate.name}\t{crate.directory}\t{feature or ''}"


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(
        description="Enforce a committed line-coverage floor for every workspace crate."
    )
    parser.add_argument(
        "--reports-dir",
        type=Path,
        default=REPO_ROOT / "coverage",
        help="directory holding <crate-name>/cobertura.xml (default: ./coverage)",
    )
    parser.add_argument(
        "--repo-root", type=Path, default=REPO_ROOT, help=argparse.SUPPRESS
    )
    parser.add_argument("--summary", type=Path, help="append the table to this file")
    parser.add_argument(
        "--github-output", type=Path, help="append per-crate step outputs to this file"
    )
    parser.add_argument(
        "--list-crates",
        action="store_true",
        help=(
            "print one tab-separated 'name<TAB>directory<TAB>feature' line per "
            "workspace crate and exit. Used by script/measure-coverage.sh so the "
            "measurement and the gate agree on what a crate is."
        ),
    )
    args = parser.parse_args(argv)

    try:
        metadata = read_metadata(args.repo_root)
        crates = workspace_crates(metadata, args.repo_root)
    except BrokenWorkspaceError as error:
        print(f"::error::{error}", file=sys.stderr)
        return 2

    if args.list_crates:
        for crate in crates:
            print(
                list_crates_line(crate, measurement_feature(crate, metadata))
            )
        return 0

    results = evaluate(crates, args.reports_dir, args.repo_root)

    if args.summary:
        write_summary(args.summary, results)
    if args.github_output:
        write_github_output(args.github_output, results)

    for result in results:
        if result.passed:
            print(
                f"{result.crate.name}: {format_percentage(result.actual)}% "
                f"(committed floor: {format_percentage(result.floor)}%)"
            )
        else:
            print(
                f"::error::{result.crate.name} [{result.status}]: {result.detail}",
                file=sys.stderr,
            )

    failing = [result for result in results if not result.passed]
    if failing:
        names = ", ".join(sorted(result.crate.name for result in failing))
        print(
            "::error::Coverage gate failed for: "
            f"{names}. Add tests, or -- if the drop is real and the new baseline "
            "is measured -- commit the measured value in the crate's "
            f"{FLOOR_FILENAME}. Lowering a floor to make a failing run pass is "
            "not a fix.",
            file=sys.stderr,
        )
        return 1

    print(f"::notice::Coverage floors met for all {len(results)} workspace crates")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
