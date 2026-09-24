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
