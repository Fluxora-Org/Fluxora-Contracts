import subprocess
from pathlib import Path


SCRIPT = Path("script/archival-canary.sh")


def test_archival_canary_dry_run():
    """Ensure the archival canary script is structurally valid bash."""
    assert SCRIPT.exists(), f"{SCRIPT} not found"
    result = subprocess.run(["bash", "-n", str(SCRIPT)], capture_output=True, text=True)
    assert result.returncode == 0, f"Bash syntax check failed: {result.stderr}"


def test_archival_canary_status_mode_submits_nothing():
    """Status mode is safe to run at any time: no keys, no transaction.

    Everything up to the round-trip branch must stay read-only, because that is
    the mode the runbook tells an operator to run on a schedule.
    """
    text = SCRIPT.read_text(encoding="utf-8")
    status_mode = text.split("if ! $ROUND_TRIP; then", 1)[0]

    assert "stellar contract invoke" not in status_mode
    assert "--send=yes" not in status_mode


def test_archival_canary_asserts_the_recorded_outcome():
    """The round trip asserts auto-restoration, not the failed read it replaced.

    §1 closed on 2026-09-28 with the finding that an invocation touching an
    archived persistent entry restores it and succeeds. A harness that still
    treated success as the failure signal would report the opposite of the
    recorded result, so the assertion has to be on the restored ledger set.
    """
    text = SCRIPT.read_text(encoding="utf-8")

    assert "--round-trip" in text
    assert "archived_soroban_entries" in text
    assert "needs an archived entry" in text


def test_archival_canary_rejects_unknown_arguments():
    result = subprocess.run(
        ["bash", str(SCRIPT), "--nonsense"], capture_output=True, text=True
    )
    assert result.returncode == 2
    assert "unknown argument" in result.stderr
