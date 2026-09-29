"""Tests for the workspace-wide coverage floor gate (issue #1873).

The gate replaced a stream-only check, so these cover the behaviour that made it
worth replacing: every crate cargo resolves is in scope, a crate with no
committed floor fails, and every crate's figure reaches the job summary.
"""

import importlib.util
import json
from pathlib import Path
import sys

import pytest


SCRIPT = Path(__file__).parents[1] / "script" / "check_coverage_floors.py"
SPEC = importlib.util.spec_from_file_location("check_coverage_floors", SCRIPT)
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


def metadata_for(packages, repo_root="/repo", features=None):
    """A cargo metadata document with the given (name, directory) packages.

    ``features`` maps a package name to the feature table cargo would report,
    which is how the gate decides whether to pass ``--features testutils``.
    """
    features = features or {}
    return {
        "packages": [
            {
                "name": name,
                "manifest_path": str(Path(repo_root) / directory / "Cargo.toml"),
                "features": features.get(name, {}),
            }
            for name, directory in packages
        ]
    }


def write_report(reports_dir, crate, rate="0.964", content=REPORT):
    report = reports_dir / crate / "cobertura.xml"
    report.parent.mkdir(parents=True, exist_ok=True)
    report.write_text(content, encoding="utf-8")
    return report


def commit_floor(repo_root, directory, value):
    floor = repo_root / directory / "coverage-floor.txt"
    floor.parent.mkdir(parents=True, exist_ok=True)
    floor.write_text(f"{value}\n", encoding="utf-8")
    return floor


# ---------------------------------------------------------------------------
# Report and floor parsing (unchanged behaviour, now module-level)
# ---------------------------------------------------------------------------


def test_read_coverage_returns_aggregate_and_sorted_modules(tmp_path):
    report = tmp_path / "cobertura.xml"
    report.write_text(REPORT, encoding="utf-8")

    actual, modules = coverage.read_coverage(report)

    assert actual == coverage.Decimal("96.4")
    assert [(module.filename, module.percentage) for module in modules] == [
        ("src/accrual.rs", coverage.Decimal("90.0")),
        ("src/lib.rs", coverage.Decimal("97.0")),
    ]


@pytest.mark.parametrize("value", ["", "101", "not-a-number"])
def test_read_floor_rejects_invalid_values(tmp_path, value):
    floor = tmp_path / "coverage-floor.txt"
    floor.write_text(value, encoding="utf-8")

    with pytest.raises(ValueError):
        coverage.read_floor(floor)


# ---------------------------------------------------------------------------
# Crate discovery
# ---------------------------------------------------------------------------


def test_workspace_crates_covers_every_resolved_member():
    metadata = metadata_for(
        [
            ("fluxora-stream", "contracts/stream"),
            ("fluxora_factory", "contracts/factory"),
            ("fluxora-archival-probe", "contracts/archival-probe"),
            ("fluxora-provenance", "tools/provenance"),
        ]
    )

    crates = coverage.workspace_crates(metadata, Path("/repo"))

    assert [crate.name for crate in crates] == [
        "fluxora-archival-probe",
        "fluxora-provenance",
        "fluxora-stream",
        "fluxora_factory",
    ]
    assert crates[0].directory == "contracts/archival-probe"


def test_workspace_crates_rejects_a_package_outside_the_repository():
    metadata = {
        "packages": [
            {
                "name": "stray",
                "manifest_path": "/elsewhere/stray/Cargo.toml",
                "features": {},
            }
        ]
    }

    with pytest.raises(coverage.BrokenWorkspaceError):
        coverage.workspace_crates(metadata, Path("/repo"))


def test_workspace_crates_rejects_empty_metadata():
    with pytest.raises(coverage.BrokenWorkspaceError):
        coverage.workspace_crates({"packages": []}, Path("/repo"))


def test_measurement_feature_is_only_passed_to_crates_that_declare_it():
    metadata = {
        "packages": [
            {
                "name": "fluxora-stream",
                "manifest_path": "/repo/contracts/stream/Cargo.toml",
                "features": {"testutils": ["soroban-sdk/testutils"]},
            },
            {
                "name": "fluxora-provenance",
                "manifest_path": "/repo/tools/provenance/Cargo.toml",
                "features": {},
            },
        ]
    }
    crates = {crate.name: crate for crate in coverage.workspace_crates(metadata, Path("/repo"))}

    assert (
        coverage.measurement_feature(crates["fluxora-stream"], metadata) == "testutils"
    )
    # A crate without the feature must be measured without the flag, otherwise
    # cargo rejects the invocation and the crate cannot be measured at all.
    assert coverage.measurement_feature(crates["fluxora-provenance"], metadata) is None


# ---------------------------------------------------------------------------
# The gate itself
# ---------------------------------------------------------------------------


def build_workspace(tmp_path, packages):
    """A temp repo root plus a matching metadata document."""
    repo_root = tmp_path / "repo"
    reports_dir = tmp_path / "coverage"
    repo_root.mkdir()
    reports_dir.mkdir()
    return repo_root, reports_dir, metadata_for(packages)


def run_gate(monkeypatch, tmp_path, repo_root, reports_dir, packages, **kwargs):
    metadata = metadata_for(packages, repo_root=repo_root)
    monkeypatch.setattr(coverage, "read_metadata", lambda root=None: metadata)
    extra = []
    for key, value in kwargs.items():
        extra.extend([f"--{key.replace('_', '-')}", str(value)])
    return coverage.main(
        [
            "--repo-root",
            str(repo_root),
            "--reports-dir",
            str(reports_dir),
            *extra,
        ]
    )


def test_gate_passes_when_every_crate_meets_its_floor(monkeypatch, tmp_path):
    packages = [
        ("fluxora-stream", "contracts/stream"),
        ("fluxora-provenance", "tools/provenance"),
    ]
    repo_root, reports_dir, _ = build_workspace(tmp_path, packages)
    for name, directory in packages:
        commit_floor(repo_root, directory, "96.4")
        write_report(reports_dir, name)

    exit_code = run_gate(
        monkeypatch, tmp_path, repo_root, reports_dir, packages
    )

    assert exit_code == 0


def test_gate_fails_when_one_crate_drops_below_its_floor(monkeypatch, tmp_path):
    """The regression this gate exists for: a test module deleted from any one
    crate must fail the run, not just the stream crate's."""
    packages = [
        ("fluxora-stream", "contracts/stream"),
        ("fluxora-provenance", "tools/provenance"),
    ]
    repo_root, reports_dir, _ = build_workspace(tmp_path, packages)
    commit_floor(repo_root, "contracts/stream", "96.4")
    write_report(reports_dir, "fluxora-stream")
    # The provenance tool's tests are gone: coverage collapses, floor does not.
    commit_floor(repo_root, "tools/provenance", "70")
    write_report(reports_dir, "fluxora-provenance", rate="0.0", content="<coverage line-rate='0.0'/>")

    exit_code = run_gate(monkeypatch, tmp_path, repo_root, reports_dir, packages)

    assert exit_code == 1


def test_gate_fails_when_a_new_crate_has_no_committed_floor(monkeypatch, tmp_path):
    """Acceptance criterion: adding a crate without a floor fails CI."""
    packages = [("fluxora-stream", "contracts/stream")]
    repo_root, reports_dir, _ = build_workspace(tmp_path, packages)
    commit_floor(repo_root, "contracts/stream", "96.4")
    write_report(reports_dir, "fluxora-stream")

    # A new crate lands with tests and a report, but nobody committed a floor.
    packages.append(("fluxora-brand-new", "contracts/brand-new"))
    write_report(reports_dir, "fluxora-brand-new")

    exit_code = run_gate(monkeypatch, tmp_path, repo_root, reports_dir, packages)

    assert exit_code == 1


def test_gate_fails_when_a_crate_has_no_report(monkeypatch, tmp_path):
    """A crate silently dropped from the measurement step must not pass by
    being absent."""
    packages = [
        ("fluxora-stream", "contracts/stream"),
        ("fluxora-provenance", "tools/provenance"),
    ]
    repo_root, reports_dir, _ = build_workspace(tmp_path, packages)
    commit_floor(repo_root, "contracts/stream", "96.4")
    write_report(reports_dir, "fluxora-stream")
    commit_floor(repo_root, "tools/provenance", "70")
    # No report written for fluxora-provenance.

    exit_code = run_gate(monkeypatch, tmp_path, repo_root, reports_dir, packages)

    assert exit_code == 1


def test_gate_reports_an_unreadable_floor_rather_than_treating_it_as_absent(
    monkeypatch, tmp_path, capsys
):
    packages = [("fluxora-stream", "contracts/stream")]
    repo_root, reports_dir, _ = build_workspace(tmp_path, packages)
    commit_floor(repo_root, "contracts/stream", "not-a-number")
    write_report(reports_dir, "fluxora-stream")

    exit_code = run_gate(monkeypatch, tmp_path, repo_root, reports_dir, packages)

    assert exit_code == 1
    assert "unreadable-floor" in capsys.readouterr().err


def test_summary_lists_every_crate_with_its_floor_and_figure(monkeypatch, tmp_path):
    packages = [
        ("fluxora-stream", "contracts/stream"),
        ("fluxora-archival-probe", "contracts/archival-probe"),
        ("fluxora-provenance", "tools/provenance"),
    ]
    repo_root, reports_dir, _ = build_workspace(tmp_path, packages)
    for name, directory in packages:
        commit_floor(repo_root, directory, "90")
        write_report(reports_dir, name)
    summary = tmp_path / "summary.md"

    exit_code = run_gate(
        monkeypatch,
        tmp_path,
        repo_root,
        reports_dir,
        packages,
        summary=summary,
    )

    assert exit_code == 0
    text = summary.read_text(encoding="utf-8")
    # Per-crate figures, not one aggregate number.
    for name in ("fluxora-stream", "fluxora-archival-probe", "fluxora-provenance"):
        assert f"`{name}`" in text
    assert "96.4%" in text
    assert "90%" in text
    # The per-module breakdown is preserved for each crate.
    assert "`src/accrual.rs`" in text


def test_summary_names_the_failing_crate(monkeypatch, tmp_path):
    packages = [("fluxora-stream", "contracts/stream")]
    repo_root, reports_dir, _ = build_workspace(tmp_path, packages)
    write_report(reports_dir, "fluxora-stream")
    summary = tmp_path / "summary.md"

    exit_code = run_gate(
        monkeypatch,
        tmp_path,
        repo_root,
        reports_dir,
        packages,
        summary=summary,
    )

    assert exit_code == 1
    text = summary.read_text(encoding="utf-8")
    assert "missing-floor" in text
    assert "contracts/stream/coverage-floor.txt" in text


def test_github_output_exposes_one_figure_per_measured_crate(monkeypatch, tmp_path):
    packages = [
        ("fluxora-stream", "contracts/stream"),
        ("fluxora-provenance", "tools/provenance"),
    ]
    repo_root, reports_dir, _ = build_workspace(tmp_path, packages)
    for name, directory in packages:
        commit_floor(repo_root, directory, "90")
        write_report(reports_dir, name)
    output = tmp_path / "github-output"

    exit_code = run_gate(
        monkeypatch,
        tmp_path,
        repo_root,
        reports_dir,
        packages,
        github_output=output,
    )

    assert exit_code == 0
    text = output.read_text(encoding="utf-8")
    assert "coverage_fluxora-stream=96.4" in text
    assert "coverage_fluxora-provenance=96.4" in text


def test_list_crates_emits_one_row_per_workspace_member(monkeypatch, tmp_path, capsys):
    packages = [("fluxora-stream", "contracts/stream"), ("fluxora-provenance", "tools/provenance")]
    metadata = metadata_for(
        packages,
        repo_root=tmp_path,
        features={"fluxora-stream": {"testutils": ["soroban-sdk/testutils"]}},
    )
    monkeypatch.setattr(coverage, "read_metadata", lambda root=None: metadata)

    exit_code = coverage.main(["--repo-root", str(tmp_path), "--list-crates"])

    assert exit_code == 0
    rows = [line for line in capsys.readouterr().out.splitlines() if line]
    assert rows == [
        "fluxora-provenance\ttools/provenance\t",
        "fluxora-stream\tcontracts/stream\ttestutils",
    ]


def test_broken_workspace_metadata_exits_two(monkeypatch, tmp_path, capsys):
    def explode(root=None):
        raise coverage.BrokenWorkspaceError("cargo metadata failed: not a workspace")

    monkeypatch.setattr(coverage, "read_metadata", explode)

    exit_code = coverage.main(["--repo-root", str(tmp_path)])

    assert exit_code == 2
    assert "::error::" in capsys.readouterr().err


# ---------------------------------------------------------------------------
# The gate is wired to the real workspace
# ---------------------------------------------------------------------------


REPO_ROOT = Path(__file__).parents[1]


def test_every_workspace_crate_has_a_committed_coverage_floor():
    """The acceptance criterion, asserted against the real workspace rather
    than a fixture: a crate added without a floor must fail here, before CI
    ever runs tarpaulin."""
    metadata = json.loads(
        coverage.subprocess.run(
            [
                "cargo",
                "metadata",
                "--format-version",
                "1",
                "--no-deps",
            ],
            cwd=REPO_ROOT,
            check=True,
            capture_output=True,
            text=True,
        ).stdout
    )
    crates = coverage.workspace_crates(metadata, REPO_ROOT)

    assert crates, "the workspace must not be empty"
    missing = [
        f"{crate.directory}/{coverage.FLOOR_FILENAME}"
        for crate in crates
        if not crate.floor_path_under(REPO_ROOT).is_file()
    ]
    assert not missing, f"workspace crates with no committed coverage floor: {missing}"
