#!/usr/bin/env python3
"""Enforce and summarize the stream contract's Cobertura coverage floor."""

from __future__ import annotations

import argparse
import os
import sys
import xml.etree.ElementTree as ET
from dataclasses import dataclass
from decimal import Decimal, InvalidOperation
from pathlib import Path


@dataclass(frozen=True)
class ModuleCoverage:
    filename: str
    percentage: Decimal


def _percentage(rate: str) -> Decimal:
    try:
        return Decimal(rate) * Decimal("100")
    except InvalidOperation as error:
        raise ValueError(f"invalid coverage rate: {rate!r}") from error


def read_coverage(xml_path: Path) -> tuple[Decimal, list[ModuleCoverage]]:
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


def _format_percentage(value: Decimal) -> str:
    return f"{value:.2f}".rstrip("0").rstrip(".")


def write_summary(
    summary_path: Path,
    actual: Decimal,
    floor: Decimal,
    modules: list[ModuleCoverage],
) -> None:
    status = "Passed" if actual >= floor else "Failed"
    lines = [
        "## Stream Contract Coverage",
        "",
        "| Metric | Value |",
        "|--------|-------|",
        f"| Line coverage | {_format_percentage(actual)}% |",
        f"| Committed floor | {_format_percentage(floor)}% |",
        f"| Status | {status} |",
        "",
        "### Coverage by Module",
        "",
        "| Module | Line coverage |",
        "|--------|---------------|",
    ]
    lines.extend(
        f"| `{module.filename}` | {_format_percentage(module.percentage)}% |"
        for module in modules
    )
    with summary_path.open("a", encoding="utf-8") as summary:
        summary.write("\n".join(lines) + "\n")


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--xml", type=Path, required=True)
    parser.add_argument("--floor", type=Path, required=True)
    parser.add_argument("--summary", type=Path)
    parser.add_argument("--github-output", type=Path)
    args = parser.parse_args(argv)

    try:
        actual, modules = read_coverage(args.xml)
        floor = read_floor(args.floor)
    except (OSError, ET.ParseError, ValueError) as error:
        print(f"::error::{error}", file=sys.stderr)
        return 1

    if args.summary:
        write_summary(args.summary, actual, floor, modules)
    if args.github_output:
        with args.github_output.open("a", encoding="utf-8") as output:
            output.write(f"coverage_pct={_format_percentage(actual)}\n")

    print(
        f"Stream contract line coverage: {_format_percentage(actual)}% "
        f"(committed floor: {_format_percentage(floor)}%)"
    )
    if actual < floor:
        print(
            "::error::Coverage is below the committed floor. "
            "Add tests or raise the floor deliberately with the measured baseline.",
            file=sys.stderr,
        )
        return 1
    print("::notice::Coverage floor passed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))