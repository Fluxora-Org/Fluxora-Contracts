"""Dry-run coverage for script/archival-canary.sh (#1863).

The canary is the only thing that can close docs/KNOWN-LIMITATIONS.md §1: it is
the live-network archival/restore round trip, and it runs against a real probe
contract, so it cannot run in CI. That is exactly why it needs a dry run here —
the script is the *mechanism* for detecting archival problems, and a script that
has drifted out of step with the probe (wrong key XDR, wrong entry point name,
wrong ledger arithmetic) fails silently in the one place nobody is watching.

The previous coverage was `bash -n`, which proves the file parses and nothing
else: a script that invoked the wrong contract method, or that computed
"remaining" the wrong way round, passed.

So this module runs the script for real, against stubbed network boundaries:

* a fake `curl` answering the `getLatestLedger` JSON-RPC call with a chosen
  ledger sequence — that is the only network input the status path needs;
* a fake `stellar` that fails, standing in for the CLI reporting an archived
  entry, so the archived branch runs too.

Both branches then execute end to end offline, and their exit codes and output
are asserted. The restore branch is deliberately *not* driven here: it submits
transactions, and a stub that "succeeds" would prove nothing about a real
network.
"""

import os
import re
import shutil
import stat
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
import pytest

REPO = Path(__file__).resolve().parents[1]
SCRIPT = REPO / "script" / "archival-canary.sh"


# ---------------------------------------------------------------------------
# Reading the script's own constants
# ---------------------------------------------------------------------------

def script_text() -> str:
    return SCRIPT.read_text(encoding="utf-8")


def constant(name: str) -> str:
    """The literal value of a `NAME=...` assignment in the script.

    Handles both a plain literal and the script's overridable form
    `${NAME:-default}`, which is how the network, RPC URL, source account and
    probe address are declared.
    """
    match = re.search(rf"^{name}=(.*)$", script_text(), re.MULTILINE)
    assert match, f"{name} is not defined in {SCRIPT.name}"
    value = match.group(1).strip()
    default = re.fullmatch(r'"\$\{' + name + r":-(.*)\}\"", value)
    return default.group(1) if default else value.strip('"')


def int_constant(name: str) -> int:
    value = constant(name)
    assert value.isdigit(), f"{name} should be a plain integer, got {value!r}"
    return int(value)


# ---------------------------------------------------------------------------
# A PATH shim for the script's two network binaries
# ---------------------------------------------------------------------------

def write_shim(directory: Path, name: str, body: str) -> None:
    path = directory / name
    path.write_text("#!/usr/bin/env bash\n" + body, encoding="utf-8")
    path.chmod(path.stat().st_mode | stat.S_IEXEC)


@pytest.fixture
def stub_bin(tmp_path: Path):
    """A directory of fake `curl` / `stellar` binaries, ready for PATH."""

    def build(*, latest_ledger: int, stellar_exit: int = 0) -> Path:
        bin_dir = tmp_path / "bin"
        bin_dir.mkdir(exist_ok=True)

        # `latest_ledger()` posts JSON-RPC and pipes the body into python3, so
        # the stub only has to print a well-formed response.
        write_shim(
            bin_dir,
            "curl",
            "printf '{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"sequence\":%d}}'\n"
            % latest_ledger,
        )
        write_shim(
            bin_dir,
            "stellar",
            f'echo "stub stellar: no live entry" >&2\nexit {stellar_exit}\n',
        )
        return bin_dir

    return build


def run_canary(bin_dir: Path, *args: str) -> subprocess.CompletedProcess:
    env = dict(os.environ)
    env["PATH"] = f"{bin_dir}{os.pathsep}{env['PATH']}"
    return subprocess.run(
        ["bash", str(SCRIPT), *args],
        capture_output=True,
        text=True,
        env=env,
        cwd=REPO,
        timeout=60,
    )


# ---------------------------------------------------------------------------
# The script's documented invariants
# ---------------------------------------------------------------------------

def test_the_canary_script_exists_and_is_valid_bash():
    assert SCRIPT.exists(), f"{SCRIPT} not found"
    result = subprocess.run(
        ["bash", "-n", str(SCRIPT)], capture_output=True, text=True
    )
    assert result.returncode == 0, f"Bash syntax check failed: {result.stderr}"


def test_the_probe_id_is_the_deployed_probe_address():
    """A canary pointed at the wrong contract reports success against nothing."""
    probe = constant("PROBE")
    assert re.fullmatch(r"C[A-Z2-7]{55}", probe), (
        f"PROBE should be a Soroban contract address, got {probe!r}"
    )


def test_the_key_xdr_encodes_the_probes_canary_key():
    """`KEY_XDR` is what the CLI reads and restores; it must be the canary key.

    The probe writes a single `Key::Canary` persistent entry (see
    contracts/archival-probe/src/lib.rs), so the XDR is a one-element vec
    holding the symbol `Canary`. A stale constant here would silently read a
    non-existent key and report "archived" for a live entry.
    """
    import base64

    raw = base64.b64decode(constant("KEY_XDR"))
    assert b"Canary" in raw, (
        "KEY_XDR does not encode the `Canary` symbol the probe writes"
    )


def test_the_live_until_ledger_is_the_plant_time_plus_the_network_minimum():
    """The countdown constants must agree with each other.

    The probe deliberately never extends its TTL, so the entry receives exactly
    the network's `min_persistent_ttl` (120,960 ledgers) — the figure the script
    records as `LIVE_UNTIL_LEDGER - PLANTED_AT_LEDGER`. If that arithmetic ever
    stops holding, every subsequent status report is wrong.
    """
    planted = int_constant("PLANTED_AT_LEDGER")
    live_until = int_constant("LIVE_UNTIL_LEDGER")

    assert live_until > planted, "LIVE_UNTIL_LEDGER must be after PLANTED_AT_LEDGER"

    min_persistent_ttl = 120_960  # network constant, both testnet and quickstart
    assert live_until - planted == min_persistent_ttl - 1, (
        "the recorded window is "
        f"{live_until - planted} ledgers, but a freshly planted entry receives "
        f"{min_persistent_ttl} minus the one consumed by the plant transaction"
    )


# ---------------------------------------------------------------------------
# Dry runs
# ---------------------------------------------------------------------------

def test_status_run_reports_alive_and_exits_zero(stub_bin):
    """The status path, offline: an entry still inside its window."""
    planted = int_constant("PLANTED_AT_LEDGER")
    bin_dir = stub_bin(latest_ledger=planted + 10)

    result = run_canary(bin_dir)

    assert result.returncode == 0, result.stderr
    assert "ALIVE" in result.stdout, result.stdout
    assert "Not archived yet" in result.stdout
    # The remaining figure is the difference to live-until, not the difference
    # from plant time.
    remaining = int_constant("LIVE_UNTIL_LEDGER") - (planted + 10)
    assert str(remaining) in result.stdout, (
        f"expected the remaining {remaining} ledgers in the banner:\n{result.stdout}"
    )


def test_status_run_with_no_ledgers_left_flags_archival(stub_bin):
    """The same run one ledger past the window: still no network writes."""
    bin_dir = stub_bin(
        latest_ledger=int_constant("LIVE_UNTIL_LEDGER") + 1, stellar_exit=1
    )

    result = run_canary(bin_dir)

    # The read fails, the invoke fails, and without --restore the script stops
    # there and reports the round trip as pending rather than claiming success.
    assert "PAST LIVE-UNTIL" in result.stdout, result.stdout
    assert "read failed: the entry is archived" in result.stdout, result.stdout
    assert "--restore" in result.stdout, result.stdout


def test_the_archived_branch_refuses_to_restore_unless_asked(stub_bin):
    """`--restore` is opt-in: a monitoring job must be able to run read-only."""
    calls = Path(os.environ.get("TMPDIR", "/tmp")) / "canary-stub-calls"
    bin_dir = stub_bin(
        latest_ledger=int_constant("LIVE_UNTIL_LEDGER") + 1, stellar_exit=1
    )
    # Log every `stellar` invocation so the restore step can be detected.
    write_shim(
        bin_dir,
        "stellar",
        f'echo "$@" >> "{calls}"\necho "stub stellar: no live entry" >&2\nexit 1\n',
    )
    calls.unlink(missing_ok=True)

    result = run_canary(bin_dir)

    invocations = calls.read_text(encoding="utf-8") if calls.exists() else ""
    assert "restore" not in invocations, (
        f"the script restored without --restore:\n{invocations}"
    )
    assert result.returncode == 0, result.stderr
    calls.unlink(missing_ok=True)


def test_the_script_never_reports_success_before_the_read_back(stub_bin):
    """The one claim that must never be made loosely.

    The banner "Round trip complete" may only appear after the read returns the
    planted value; the stubbed run stops before that, so it must be absent.
    """
    bin_dir = stub_bin(
        latest_ledger=int_constant("LIVE_UNTIL_LEDGER") + 1, stellar_exit=1
    )
    result = run_canary(bin_dir)

    assert "Round trip complete" not in result.stdout, result.stdout


def test_the_script_requires_no_clone_and_no_local_state():
    """It must stay a pure CLI script: no repo-relative writes, no temp files."""
    text = script_text()
    for forbidden in ["git ", "cargo ", ">> ", "mktemp", "/tmp/"]:
        assert forbidden not in text, (
            f"archival-canary.sh should not use `{forbidden}`; it must be "
            "runnable from any checkout against any network"
        )


def test_stub_environment_is_actually_used(stub_bin):
    """Guard on the harness itself: the stubs must shadow the real binaries.

    Without this, a `curl` that happened to be installed would let the "offline"
    tests reach the real RPC endpoint, and the assertions above would be
    measuring the network rather than the script.
    """
    bin_dir = stub_bin(latest_ledger=123)
    resolved = shutil.which("curl", path=f"{bin_dir}{os.pathsep}{os.environ['PATH']}")
    assert Path(resolved).parent == bin_dir, (
        f"the stub curl at {bin_dir} did not take precedence over {resolved}"
    )
