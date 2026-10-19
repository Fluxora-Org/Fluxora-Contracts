from pathlib import Path


WORKFLOW = Path(__file__).parents[1] / ".github" / "workflows" / "ci.yml"
WORKFLOW_TEXT = WORKFLOW.read_text(encoding="utf-8")


def test_python_tooling_job_runs_unconditionally_with_coverage_report():
    job = WORKFLOW_TEXT.split("  lint:\n", maxsplit=1)[0]

    assert "docs-alignment-check:" in job
    assert "if: ${{ hashFiles('tests/**'" not in job
    assert "pytest tests/ --cov=script/ --cov-fail-under=50" in job
    assert "--cov-report=term-missing" in job
    assert "--cov-report=xml:coverage/python-tooling.xml" in job
    assert "name: python-tooling-coverage" in job


def test_event_snapshot_gate_is_a_ci_step():
    """Issue #1701: the event snapshot gate must run as a hard CI step."""
    job = WORKFLOW_TEXT.split("  lint:\n", maxsplit=1)[0]

    assert "Event snapshot coverage gate" in job
    assert "run: python3 script/check_event_snapshots.py" in job
    # It must be a hard gate, not a tolerated failure.
    gate = job.split("- name: Event snapshot coverage gate", maxsplit=1)[1]
    step = gate.split("- name:", maxsplit=1)[0]
    assert "continue-on-error" not in step
    # A failure in an earlier step of the job must not mask the verdict.
    assert "if: always()" in step
