import json
import os
import stat
import subprocess
import tempfile
import pytest
from unittest.mock import patch
from script.validate_gas import build_cargo_test_env, extract_baselines, parse_measurements, main


# main() validates that these mandatory matrix entries exist before comparing
# measurements. Include them in focused mocks so each test reaches the behavior
# it intends to exercise instead of failing during baseline-shape validation.
REQUIRED_BULK_FIXTURE = {
    "bulk_cancel_streams": {"1": 1, "5": 1, "10": 1, "20": 1},
    "bulk_resume_streams_as_admin": {"1": 1, "5": 1, "10": 1, "20": 1},
}


class TestBuildCargoTestEnv:
    def test_prepends_cargo_bin(self, monkeypatch):
        monkeypatch.setenv("HOME", "/home/ci")
        monkeypatch.setenv("PATH", "/bin")
        assert build_cargo_test_env()["PATH"] == "/home/ci/.cargo/bin:/bin"

    def test_missing_home_raises(self, monkeypatch):
        monkeypatch.delenv("HOME", raising=False)
        with pytest.raises(RuntimeError, match="HOME is not set"):
            build_cargo_test_env()


class TestExtractBaselines:
    def test_extract_baselines_success(self):
        """Test successful extraction of gas baselines from markdown."""
        content = """
        # Gas Documentation
        <!-- GAS_BASELINE_START -->
        {"batch_withdraw": {"single": 1000}, "transfer": 2000}
        <!-- GAS_BASELINE_END -->
        """
        with patch("builtins.open", create=True) as mock_file:
            mock_file.return_value.__enter__.return_value.read.return_value = content
            result = extract_baselines("docs/gas.md")
            assert result == {"batch_withdraw": {"single": 1000}, "transfer": 2000}

    def test_extract_baselines_missing_block(self):
        """Test error when baseline block is missing."""
        content = "# Gas Documentation\nNo baseline here"
        with patch("builtins.open", create=True) as mock_file:
            mock_file.return_value.__enter__.return_value.read.return_value = content
            with pytest.raises(ValueError, match="Could not find gas baseline block"):
                extract_baselines("docs/gas.md")


class TestParseMeasurements:
    def test_parse_measurements_valid(self):
        """Test parsing valid gas measurement output."""
        output = """
        GAS_MEASUREMENT: batch_withdraw: single: 1050
        GAS_MEASUREMENT: transfer: single: 2100
        """
        result = parse_measurements(output)
        assert result == {
            "batch_withdraw": {"single": 1050},
            "transfer": {"single": 2100},
        }

    def test_parse_measurements_empty(self):
        """Test parsing output with no measurements."""
        output = "No measurements found"
        result = parse_measurements(output)
        assert result == {}

    def test_parse_measurements_multiple_sizes(self):
        """Test parsing multiple size variants."""
        output = """
        GAS_MEASUREMENT: batch_withdraw: small: 1000
        GAS_MEASUREMENT: batch_withdraw: large: 5000
        """
        result = parse_measurements(output)
        assert result == {
            "batch_withdraw": {"small": 1000, "large": 5000}
        }


class TestMain:
    @patch("script.validate_gas.run_tests")
    @patch("script.validate_gas.extract_baselines")
    @patch("script.validate_gas.sys.exit")
    def test_main_no_regressions(self, mock_exit, mock_baselines, mock_run_tests):
        """Test successful validation with no regressions."""
        mock_baselines.return_value = {
            "transfer": 2000,
            "bulk_cancel_streams": {"1": 3000, "5": 7000, "10": 12000, "20": 22000},
            "bulk_resume_streams_as_admin": {"1": 4000, "5": 8000, "10": 14000, "20": 26000},
        }
        mock_run_tests.return_value = "GAS_MEASUREMENT: transfer: single: 1900"
        main()
        mock_exit.assert_called_with(0)

    @patch("script.validate_gas.run_tests")
    @patch("script.validate_gas.extract_baselines")
    @patch("script.validate_gas.sys.exit")
    def test_main_with_regression(self, mock_exit, mock_baselines, mock_run_tests):
        """Test failure when gas regression is detected."""
        mock_baselines.return_value = {**REQUIRED_BULK_FIXTURE, "transfer": 1000}
        mock_run_tests.return_value = "GAS_MEASUREMENT: transfer: single: 1100"
        main()
        mock_exit.assert_called_with(1)

    @patch("script.validate_gas.run_tests")
    @patch("script.validate_gas.extract_baselines")
    @patch("script.validate_gas.sys.exit")
    def test_main_no_measurements(self, mock_exit, mock_baselines, mock_run_tests):
        """Test error when no measurements found."""
        mock_baselines.return_value = {**REQUIRED_BULK_FIXTURE, "transfer": 2000}
        mock_run_tests.return_value = "No measurements"
        main()
        mock_exit.assert_any_call(1)

    @patch("script.validate_gas.extract_baselines")
    @patch("script.validate_gas.sys.exit")
    def test_main_exception_handling(self, mock_exit, mock_baselines):
        """Test exception handling."""
        mock_baselines.side_effect = Exception("Test error")
        main()
        mock_exit.assert_called_with(1)

    @patch("script.validate_gas.run_tests")
    @patch("script.validate_gas.extract_baselines")
    @patch("script.validate_gas.sys.exit")
    def test_main_exactly_five_percent_increase_fails(
        self, mock_exit, mock_baselines, mock_run_tests
    ):
        """A measurement that is exactly +5.0% above the baseline should FAIL.

        The tolerance gate is ``diff > 0.05`` which is a strict greater-than
        comparison.  A 5.0 % increase means diff == 0.05, which is NOT > 0.05,
        so the boundary case should PASS.  This test locks down that exact
        semantic so a future change to ``>=`` would be caught immediately.
        """
        baseline = 1000
        # Exactly +5 % → diff = 0.05 exactly, NOT > 0.05 → should PASS
        exactly_five_pct = int(baseline * 1.05)
        mock_baselines.return_value = {**REQUIRED_BULK_FIXTURE, "withdraw": baseline}
        mock_run_tests.return_value = (
            f"GAS_MEASUREMENT: withdraw: single: {exactly_five_pct}"
        )
        main()
        mock_exit.assert_called_with(0)

    @patch("script.validate_gas.run_tests")
    @patch("script.validate_gas.extract_baselines")
    @patch("script.validate_gas.sys.exit")
    def test_main_one_unit_above_five_percent_fails(
        self, mock_exit, mock_baselines, mock_run_tests
    ):
        """A measurement one instruction above the +5 % threshold should FAIL.

        baseline=1000, threshold boundary = 1050 (PASS), 1051 (FAIL).
        """
        baseline = 1000
        one_over = int(baseline * 1.05) + 1  # 1051
        mock_baselines.return_value = {**REQUIRED_BULK_FIXTURE, "withdraw": baseline}
        mock_run_tests.return_value = (
            f"GAS_MEASUREMENT: withdraw: single: {one_over}"
        )
        main()
        mock_exit.assert_called_with(1)

    @patch("script.validate_gas.run_tests")
    @patch("script.validate_gas.extract_baselines")
    @patch("script.validate_gas.sys.exit")
    def test_main_missing_baseline_key_prints_missing_not_fail(
        self, mock_exit, mock_baselines, mock_run_tests
    ):
        """A measured function whose key is absent from the baseline block
        should be printed as MISSING, not counted as a regression.

        The script currently outputs MISSING and continues without failing CI.
        This test locks down that behaviour so a future change that turns
        missing keys into failures is caught explicitly.
        """
        # Baseline has "create_stream" but the test output reports "new_op".
        mock_baselines.return_value = {**REQUIRED_BULK_FIXTURE, "create_stream": 500000}
        mock_run_tests.return_value = (
            "GAS_MEASUREMENT: new_op: single: 100000"
        )
        main()
        # Missing key → MISSING row, no regression → should exit 0 after
        # printing the MISSING line (not exit 1).
        mock_exit.assert_called_with(0)

    @patch("script.validate_gas.run_tests")
    @patch("script.validate_gas.extract_baselines")
    @patch("script.validate_gas.sys.exit")
    def test_main_nested_dict_baseline_lookup_keeper_cancel(
        self, mock_exit, mock_baselines, mock_run_tests
    ):
        """keeper_cancel uses a nested dict baseline; both variants must resolve
        correctly and not produce a MISSING row.

        The baseline structure is:
            {"keeper_cancel": {"partial_accrual": 786739, "fully_accrued": 386889}}

        The GAS_MEASUREMENT lines use "partial_accrual" and "fully_accrued" as
        the size/variant key.  The lookup in validate_gas.main() should match
        ``baselines["keeper_cancel"]["partial_accrual"]`` etc.
        """
        mock_baselines.return_value = {
            **REQUIRED_BULK_FIXTURE,
            "keeper_cancel": {
                "partial_accrual": 786739,
                "fully_accrued": 386889,
            }
        }
        # Both variants well within +5 % → should PASS
        mock_run_tests.return_value = (
            "GAS_MEASUREMENT: keeper_cancel: partial_accrual: 786739\n"
            "GAS_MEASUREMENT: keeper_cancel: fully_accrued: 386889\n"
        )
        main()
        mock_exit.assert_called_with(0)

    @patch("script.validate_gas.run_tests")
    @patch("script.validate_gas.extract_baselines")
    @patch("script.validate_gas.sys.exit")
    def test_main_flat_int_baseline_lookup(
        self, mock_exit, mock_baselines, mock_run_tests
    ):
        """create_stream and withdraw use a flat integer baseline; the lookup
        path for flat ints must work alongside the nested-dict path.

        Regression guard: a refactor that always dereferences baselines as a
        dict would break flat-int lookup.
        """
        mock_baselines.return_value = {
            **REQUIRED_BULK_FIXTURE,
            "create_stream": 568292,
            "withdraw": 562057,
        }
        mock_run_tests.return_value = (
            "GAS_MEASUREMENT: create_stream: single: 560000\n"
            "GAS_MEASUREMENT: withdraw: single: 555000\n"
        )
        main()
        mock_exit.assert_called_with(0)


# ---------------------------------------------------------------------------
# WASM size budget tests — exercise script/check-wasm-size.sh
# ---------------------------------------------------------------------------

SCRIPT = os.path.join(os.path.dirname(__file__), "..", "script", "check-wasm-size.sh")
WASM_DIR = "target/wasm32-unknown-unknown/release"


def _make_wasm(directory: str, name: str, size_bytes: int) -> str:
    """Create a dummy WASM file of exactly size_bytes bytes."""
    path = os.path.join(directory, name)
    with open(path, "wb") as f:
        f.write(b"\x00" * size_bytes)
    return path


class TestCheckWasmSizeScript:
    """Tests for script/check-wasm-size.sh."""

    def _invoke(self, wasm_dir: str, optimized: bool = False) -> subprocess.CompletedProcess:
        args = ["--optimized"] if optimized else []
        env = {**os.environ, "GITHUB_STEP_SUMMARY": "", "WASM_DIR": wasm_dir}
        return subprocess.run(
            ["bash", SCRIPT] + args,
            capture_output=True, text=True, env=env,
        )

    def test_all_within_budget_passes(self, tmp_path):
        """All contracts under budget → exit 0."""
        for contract, budget in [
            ("fluxora_stream", 262144),
            ("fluxora_factory", 131072),
            ("fluxora_governance", 131072),
        ]:
            _make_wasm(str(tmp_path), f"{contract}.wasm", budget - 1)

        result = self._invoke(str(tmp_path))
        assert result.returncode == 0, result.stderr
        assert "All contracts within WASM size budget" in result.stdout

    def test_stream_over_budget_fails(self, tmp_path):
        """stream contract over budget → exit 1."""
        _make_wasm(str(tmp_path), "fluxora_stream.wasm", 262145)   # 1 byte over
        _make_wasm(str(tmp_path), "fluxora_factory.wasm", 1024)
        _make_wasm(str(tmp_path), "fluxora_governance.wasm", 1024)

        result = self._invoke(str(tmp_path))
        assert result.returncode == 1
        assert "OVER BUDGET" in result.stderr or "exceeds budget" in result.stderr

    def test_factory_over_budget_fails(self, tmp_path):
        """factory contract over budget → exit 1."""
        _make_wasm(str(tmp_path), "fluxora_stream.wasm", 1024)
        _make_wasm(str(tmp_path), "fluxora_factory.wasm", 131073)  # 1 byte over
        _make_wasm(str(tmp_path), "fluxora_governance.wasm", 1024)

        result = self._invoke(str(tmp_path))
        assert result.returncode == 1

    def test_governance_over_budget_fails(self, tmp_path):
        """governance contract over budget → exit 1."""
        _make_wasm(str(tmp_path), "fluxora_stream.wasm", 1024)
        _make_wasm(str(tmp_path), "fluxora_factory.wasm", 1024)
        _make_wasm(str(tmp_path), "fluxora_governance.wasm", 131073)  # 1 byte over

        result = self._invoke(str(tmp_path))
        assert result.returncode == 1

    def test_missing_artifact_fails(self, tmp_path):
        """Missing artifact → exit 1 with error message."""
        # Only create two of the three contracts.
        _make_wasm(str(tmp_path), "fluxora_stream.wasm", 1024)
        _make_wasm(str(tmp_path), "fluxora_factory.wasm", 1024)
        # fluxora_governance.wasm intentionally absent.

        result = self._invoke(str(tmp_path))
        assert result.returncode == 1
        assert "not found" in result.stderr or "MISSING" in result.stderr

    def test_exact_budget_boundary_passes(self, tmp_path):
        """Artifact exactly at budget → still passes (budget is inclusive)."""
        for contract, budget in [
            ("fluxora_stream", 262144),
            ("fluxora_factory", 131072),
            ("fluxora_governance", 131072),
        ]:
            _make_wasm(str(tmp_path), f"{contract}.wasm", budget)

        result = self._invoke(str(tmp_path))
        assert result.returncode == 0, result.stderr

    def test_optimized_flag_checks_optimized_files(self, tmp_path):
        """--optimized flag reads *.optimized.wasm files."""
        for contract, budget in [
            ("fluxora_stream", 262144),
            ("fluxora_factory", 131072),
            ("fluxora_governance", 131072),
        ]:
            _make_wasm(str(tmp_path), f"{contract}.optimized.wasm", budget - 1)

        result = self._invoke(str(tmp_path), optimized=True)
        assert result.returncode == 0, result.stderr

    def test_script_is_executable(self):
        """Script file has executable bit set."""
        mode = os.stat(SCRIPT).st_mode
        assert mode & stat.S_IXUSR, "check-wasm-size.sh is not executable"

    def test_headroom_reported_in_stdout(self, tmp_path):
        """Passing run reports headroom for each contract."""
        for contract, budget in [
            ("fluxora_stream", 262144),
            ("fluxora_factory", 131072),
            ("fluxora_governance", 131072),
        ]:
            _make_wasm(str(tmp_path), f"{contract}.wasm", budget // 2)

        result = self._invoke(str(tmp_path))
        assert result.returncode == 0
        assert "headroom" in result.stdout

    def test_unknown_flag_exits_nonzero(self, tmp_path):
        """Passing an unknown flag → exit 1."""
        env = {**os.environ, "GITHUB_STEP_SUMMARY": ""}
        result = subprocess.run(
            ["bash", SCRIPT, "--unknown-flag"],
            capture_output=True, text=True, env=env,
        )
        assert result.returncode == 1

    def test_headroom_computation_matches_formula(self, tmp_path):
        """Headroom reported in stdout equals (budget - actual_size) bytes.

        This test locks down the formula: reported headroom must exactly match
        the arithmetic difference between the budget constant and the file size.
        It verifies that the script does not round, truncate, or re-compute the
        headroom differently from the simple subtraction.

        Budget constants (from check-wasm-size.sh):
            fluxora_stream:     262 144 bytes
            fluxora_factory:    131 072 bytes
            fluxora_governance: 131 072 bytes
        """
        stream_size = 200_000       # headroom = 262144 - 200000 = 62144
        factory_size = 100_000      # headroom = 131072 - 100000 = 31072
        governance_size = 90_000    # headroom = 131072 -  90000 = 41072

        _make_wasm(str(tmp_path), "fluxora_stream.wasm", stream_size)
        _make_wasm(str(tmp_path), "fluxora_factory.wasm", factory_size)
        _make_wasm(str(tmp_path), "fluxora_governance.wasm", governance_size)

        result = self._invoke(str(tmp_path))
        assert result.returncode == 0, result.stderr

        stream_headroom = 262_144 - stream_size          # 62 144
        factory_headroom = 131_072 - factory_size        # 31 072
        governance_headroom = 131_072 - governance_size  # 41 072

        # The script formats headroom in KiB or bytes; check for the KiB value
        # (rounded) or the raw byte count. We accept either representation.
        def _kib(b: int) -> str:
            return f"{b / 1024:.1f} KiB"

        stdout = result.stdout
        for headroom in (stream_headroom, factory_headroom, governance_headroom):
            assert (
                str(headroom) in stdout or _kib(headroom) in stdout
            ), (
                f"Expected headroom {headroom} bytes ({_kib(headroom)}) "
                f"in stdout, got:\n{stdout}"
            )

    def test_all_at_max_minus_one_all_pass(self, tmp_path):
        """All artifacts one byte under budget → all pass, full headroom of 1."""
        for contract, budget in [
            ("fluxora_stream", 262144),
            ("fluxora_factory", 131072),
            ("fluxora_governance", 131072),
        ]:
            _make_wasm(str(tmp_path), f"{contract}.wasm", budget - 1)

        result = self._invoke(str(tmp_path))
        assert result.returncode == 0, result.stderr
        # Each contract has headroom of 1 byte
        assert result.stdout.count("headroom") >= 3

    def test_stream_exactly_at_budget_boundary(self, tmp_path):
        """Stream contract artifact exactly at its 256 KiB budget passes.

        Regression guard for an off-by-one in the ``<=`` comparison in the
        script.  If the script were ``< budget`` instead of ``<= budget``,
        a file at exactly the budget would incorrectly fail.
        """
        _make_wasm(str(tmp_path), "fluxora_stream.wasm", 262144)
        _make_wasm(str(tmp_path), "fluxora_factory.wasm", 1024)
        _make_wasm(str(tmp_path), "fluxora_governance.wasm", 1024)

        result = self._invoke(str(tmp_path))
        assert result.returncode == 0, (
            "Artifact at exact budget should pass (budget check is <=), "
            f"got exit {result.returncode}\nstderr: {result.stderr}"
        )

    def test_factory_exactly_at_budget_boundary(self, tmp_path):
        """Factory contract artifact exactly at its 128 KiB budget passes."""
        _make_wasm(str(tmp_path), "fluxora_stream.wasm", 1024)
        _make_wasm(str(tmp_path), "fluxora_factory.wasm", 131072)
        _make_wasm(str(tmp_path), "fluxora_governance.wasm", 1024)

        result = self._invoke(str(tmp_path))
        assert result.returncode == 0, result.stderr

    def test_governance_exactly_at_budget_boundary(self, tmp_path):
        """Governance contract artifact exactly at its 128 KiB budget passes."""
        _make_wasm(str(tmp_path), "fluxora_stream.wasm", 1024)
        _make_wasm(str(tmp_path), "fluxora_factory.wasm", 1024)
        _make_wasm(str(tmp_path), "fluxora_governance.wasm", 131072)

        result = self._invoke(str(tmp_path))
        assert result.returncode == 0, result.stderr

    def test_all_over_budget_reports_all_failures(self, tmp_path):
        """When all three contracts exceed budget, all three failures appear."""
        _make_wasm(str(tmp_path), "fluxora_stream.wasm", 262145)
        _make_wasm(str(tmp_path), "fluxora_factory.wasm", 131073)
        _make_wasm(str(tmp_path), "fluxora_governance.wasm", 131073)

        result = self._invoke(str(tmp_path))
        assert result.returncode == 1
        # All three contracts should be mentioned in the failure output.
        assert "fluxora_stream" in result.stderr or "fluxora_stream" in result.stdout
        assert "fluxora_factory" in result.stderr or "fluxora_factory" in result.stdout
        assert "fluxora_governance" in result.stderr or "fluxora_governance" in result.stdout


# ---------------------------------------------------------------------------
# WASM size delta reporting tests — exercise delta logic in check-wasm-size.sh
# ---------------------------------------------------------------------------


def _init_git_repo(directory: str) -> None:
    """Initialize a git repo and configure user for commits."""
    subprocess.run(["git", "init"], cwd=directory, capture_output=True, check=True)
    subprocess.run(
        ["git", "config", "user.email", "test@test.com"],
        cwd=directory, capture_output=True, check=True,
    )
    subprocess.run(
        ["git", "config", "user.name", "Test"],
        cwd=directory, capture_output=True, check=True,
    )


def _git_commit(directory: str, message: str) -> None:
    """Stage all and commit."""
    subprocess.run(["git", "add", "-A"], cwd=directory, capture_output=True, check=True)
    subprocess.run(
        ["git", "commit", "-m", message, "--allow-empty"],
        cwd=directory, capture_output=True, check=True,
    )


def _git_tag(directory: str, tag: str) -> None:
    """Create an annotated tag."""
    subprocess.run(
        ["git", "tag", "-a", tag, "-m", f"Release {tag}"],
        cwd=directory, capture_output=True, check=True,
    )


class TestWasmSizeDeltaReporting:
    """Tests for per-PR WASM size delta reporting in check-wasm-size.sh."""

    def _invoke(
        self, wasm_dir: str, git_dir: str, optimized: bool = False,
        github_actions: str = "",
    ) -> subprocess.CompletedProcess:
        args = ["--optimized"] if optimized else []
        env = {
            **os.environ,
            "GITHUB_STEP_SUMMARY": "",
            "WASM_DIR": wasm_dir,
            "GIT_REPO": git_dir,
        }
        if github_actions:
            env["GITHUB_ACTIONS"] = github_actions
        return subprocess.run(
            ["bash", SCRIPT] + args,
            capture_output=True, text=True, env=env,
            cwd=git_dir,
        )

    def test_delta_positive_growth(self, tmp_path):
        """Positive delta (bloat) is reported when WASM grew since last tag."""
        # Create a git repo with a tagged release containing smaller WASM.
        git_dir = str(tmp_path / "repo")
        os.makedirs(git_dir)
        _init_git_repo(git_dir)

        # Create initial commit and tag with smaller WASM files.
        wasm_dir = os.path.join(git_dir, "target", "wasm32-unknown-unknown", "release")
        os.makedirs(wasm_dir)
        for contract, size in [
            ("fluxora_stream", 200000),
            ("fluxora_factory", 100000),
            ("fluxora_governance", 100000),
        ]:
            _make_wasm(wasm_dir, f"{contract}.wasm", size)
        _git_commit(git_dir, "baseline")
        _git_tag(git_dir, "v1.0.0")

        # Now create larger WASM files (simulate PR bloat).
        for contract, size in [
            ("fluxora_stream", 210000),   # +10000 bytes
            ("fluxora_factory", 105000),  # +5000 bytes
            ("fluxora_governance", 102000),  # +2000 bytes
        ]:
            _make_wasm(wasm_dir, f"{contract}.wasm", size)
        _git_commit(git_dir, "feature: add bloat")

        result = self._invoke(wasm_dir, git_dir)
        assert result.returncode == 0
        assert "Previous release tag: v1.0.0" in result.stdout
        assert "+10000 bytes" in result.stdout or "+9.8 KiB" in result.stdout
        assert "+5000 bytes" in result.stdout or "+4.9 KiB" in result.stdout

    def test_delta_negative_shrink(self, tmp_path):
        """Negative delta (shrink) is reported when WASM shrunk since last tag."""
        git_dir = str(tmp_path / "repo")
        os.makedirs(git_dir)
        _init_git_repo(git_dir)

        wasm_dir = os.path.join(git_dir, "target", "wasm32-unknown-unknown", "release")
        os.makedirs(wasm_dir)
        for contract, size in [
            ("fluxora_stream", 250000),
            ("fluxora_factory", 120000),
            ("fluxora_governance", 120000),
        ]:
            _make_wasm(wasm_dir, f"{contract}.wasm", size)
        _git_commit(git_dir, "baseline")
        _git_tag(git_dir, "v2.0.0")

        # Now create smaller WASM files (simulate optimization).
        for contract, size in [
            ("fluxora_stream", 240000),   # -10000 bytes
            ("fluxora_factory", 115000),  # -5000 bytes
            ("fluxora_governance", 118000),  # -2000 bytes
        ]:
            _make_wasm(wasm_dir, f"{contract}.wasm", size)
        _git_commit(git_dir, "refactor: optimize")

        result = self._invoke(wasm_dir, git_dir)
        assert result.returncode == 0
        assert "Previous release tag: v2.0.0" in result.stdout
        assert "-10000 bytes" in result.stdout or "-9.8 KiB" in result.stdout

    def test_delta_no_change(self, tmp_path):
        """Zero delta is reported when WASM is unchanged since last tag."""
        git_dir = str(tmp_path / "repo")
        os.makedirs(git_dir)
        _init_git_repo(git_dir)

        wasm_dir = os.path.join(git_dir, "target", "wasm32-unknown-unknown", "release")
        os.makedirs(wasm_dir)
        for contract, size in [
            ("fluxora_stream", 200000),
            ("fluxora_factory", 100000),
            ("fluxora_governance", 100000),
        ]:
            _make_wasm(wasm_dir, f"{contract}.wasm", size)
        _git_commit(git_dir, "baseline")
        _git_tag(git_dir, "v3.0.0")

        # Create identical WASM files (no change).
        for contract, size in [
            ("fluxora_stream", 200000),
            ("fluxora_factory", 100000),
            ("fluxora_governance", 100000),
        ]:
            _make_wasm(wasm_dir, f"{contract}.wasm", size)
        _git_commit(git_dir, "chore: no-op")

        result = self._invoke(wasm_dir, git_dir)
        assert result.returncode == 0
        assert "no change" in result.stdout

    def test_no_previous_tag(self, tmp_path):
        """Graceful handling when no release tag exists."""
        git_dir = str(tmp_path / "repo")
        os.makedirs(git_dir)
        _init_git_repo(git_dir)

        wasm_dir = os.path.join(git_dir, "target", "wasm32-unknown-unknown", "release")
        os.makedirs(wasm_dir)
        for contract, budget in [
            ("fluxora_stream", 262144),
            ("fluxora_factory", 131072),
            ("fluxora_governance", 131072),
        ]:
            _make_wasm(wasm_dir, f"{contract}.wasm", budget - 1)
        _git_commit(git_dir, "initial")

        result = self._invoke(wasm_dir, git_dir)
        assert result.returncode == 0
        assert "No previous release tag found" in result.stdout

    def test_previous_tag_no_baseline(self, tmp_path):
        """Graceful handling when previous tag doesn't have WASM file."""
        git_dir = str(tmp_path / "repo")
        os.makedirs(git_dir)
        _init_git_repo(git_dir)

        wasm_dir = os.path.join(git_dir, "target", "wasm32-unknown-unknown", "release")
        os.makedirs(wasm_dir)

        # Create a commit with only a README (no WASM files).
        with open(os.path.join(git_dir, "README.md"), "w") as f:
            f.write("# Test\n")
        _git_commit(git_dir, "initial")
        _git_tag(git_dir, "v0.1.0")

        # Now create WASM files for current build.
        for contract, budget in [
            ("fluxora_stream", 262144),
            ("fluxora_factory", 131072),
            ("fluxora_governance", 131072),
        ]:
            _make_wasm(wasm_dir, f"{contract}.wasm", budget - 1)
        _git_commit(git_dir, "add wasm")

        result = self._invoke(wasm_dir, git_dir)
        assert result.returncode == 0
        assert "baseline not available" in result.stdout

    def test_ci_annotations_positive_delta(self, tmp_path):
        """CI annotations are emitted for positive delta in GitHub Actions."""
        git_dir = str(tmp_path / "repo")
        os.makedirs(git_dir)
        _init_git_repo(git_dir)

        wasm_dir = os.path.join(git_dir, "target", "wasm32-unknown-unknown", "release")
        os.makedirs(wasm_dir)
        for contract, size in [
            ("fluxora_stream", 200000),
            ("fluxora_factory", 100000),
            ("fluxora_governance", 100000),
        ]:
            _make_wasm(wasm_dir, f"{contract}.wasm", size)
        _git_commit(git_dir, "baseline")
        _git_tag(git_dir, "v1.0.0")

        for contract, size in [
            ("fluxora_stream", 210000),
            ("fluxora_factory", 105000),
            ("fluxora_governance", 102000),
        ]:
            _make_wasm(wasm_dir, f"{contract}.wasm", size)
        _git_commit(git_dir, "bloat")

        result = self._invoke(wasm_dir, git_dir, github_actions="true")
        assert result.returncode == 0
        assert "::warning::" in result.stdout

    def test_ci_annotations_negative_delta(self, tmp_path):
        """CI notice annotations are emitted for negative delta."""
        git_dir = str(tmp_path / "repo")
        os.makedirs(git_dir)
        _init_git_repo(git_dir)

        wasm_dir = os.path.join(git_dir, "target", "wasm32-unknown-unknown", "release")
        os.makedirs(wasm_dir)
        for contract, size in [
            ("fluxora_stream", 250000),
            ("fluxora_factory", 120000),
            ("fluxora_governance", 120000),
        ]:
            _make_wasm(wasm_dir, f"{contract}.wasm", size)
        _git_commit(git_dir, "baseline")
        _git_tag(git_dir, "v1.0.0")

        for contract, size in [
            ("fluxora_stream", 240000),
            ("fluxora_factory", 115000),
            ("fluxora_governance", 118000),
        ]:
            _make_wasm(wasm_dir, f"{contract}.wasm", size)
        _git_commit(git_dir, "optimize")

        result = self._invoke(wasm_dir, git_dir, github_actions="true")
        assert result.returncode == 0
        assert "::notice::" in result.stdout

    def test_step_summary_includes_delta_table(self, tmp_path):
        """Step summary includes the delta table when baseline is available."""
        git_dir = str(tmp_path / "repo")
        os.makedirs(git_dir)
        _init_git_repo(git_dir)

        wasm_dir = os.path.join(git_dir, "target", "wasm32-unknown-unknown", "release")
        os.makedirs(wasm_dir)
        for contract, size in [
            ("fluxora_stream", 200000),
            ("fluxora_factory", 100000),
            ("fluxora_governance", 100000),
        ]:
            _make_wasm(wasm_dir, f"{contract}.wasm", size)
        _git_commit(git_dir, "baseline")
        _git_tag(git_dir, "v1.0.0")

        for contract, size in [
            ("fluxora_stream", 210000),
            ("fluxora_factory", 105000),
            ("fluxora_governance", 102000),
        ]:
            _make_wasm(wasm_dir, f"{contract}.wasm", size)
        _git_commit(git_dir, "bloat")

        summary_file = os.path.join(git_dir, "summary.md")
        env = {
            **os.environ,
            "GITHUB_STEP_SUMMARY": summary_file,
            "WASM_DIR": wasm_dir,
            "GIT_REPO": git_dir,
        }
        subprocess.run(
            ["bash", SCRIPT],
            capture_output=True, text=True, env=env, cwd=git_dir,
        )

        assert os.path.exists(summary_file)
        with open(summary_file) as f:
            content = f.read()
        assert "WASM Size Delta" in content
        assert "v1.0.0" in content

    def test_step_summary_no_delta_without_tag(self, tmp_path):
        """Step summary omits delta table when no release tag exists."""
        git_dir = str(tmp_path / "repo")
        os.makedirs(git_dir)
        _init_git_repo(git_dir)

        wasm_dir = os.path.join(git_dir, "target", "wasm32-unknown-unknown", "release")
        os.makedirs(wasm_dir)
        for contract, budget in [
            ("fluxora_stream", 262144),
            ("fluxora_factory", 131072),
            ("fluxora_governance", 131072),
        ]:
            _make_wasm(wasm_dir, f"{contract}.wasm", budget - 1)
        _git_commit(git_dir, "initial")

        summary_file = os.path.join(git_dir, "summary.md")
        env = {
            **os.environ,
            "GITHUB_STEP_SUMMARY": summary_file,
            "WASM_DIR": wasm_dir,
            "GIT_REPO": git_dir,
        }
        subprocess.run(
            ["bash", SCRIPT],
            capture_output=True, text=True, env=env, cwd=git_dir,
        )

        assert os.path.exists(summary_file)
        with open(summary_file) as f:
            content = f.read()
        assert "WASM Size Delta" not in content

    def test_optimized_delta_uses_optimized_files(self, tmp_path):
        """--optimized flag compares optimized WASM files for delta."""
        git_dir = str(tmp_path / "repo")
        os.makedirs(git_dir)
        _init_git_repo(git_dir)

        wasm_dir = os.path.join(git_dir, "target", "wasm32-unknown-unknown", "release")
        os.makedirs(wasm_dir)
        for contract, size in [
            ("fluxora_stream", 180000),
            ("fluxora_factory", 90000),
            ("fluxora_governance", 90000),
        ]:
            _make_wasm(wasm_dir, f"{contract}.optimized.wasm", size)
        _git_commit(git_dir, "baseline")
        _git_tag(git_dir, "v1.0.0")

        for contract, size in [
            ("fluxora_stream", 190000),
            ("fluxora_factory", 95000),
            ("fluxora_governance", 92000),
        ]:
            _make_wasm(wasm_dir, f"{contract}.optimized.wasm", size)
        _git_commit(git_dir, "optimized bloat")

        result = self._invoke(wasm_dir, git_dir, optimized=True)
        assert result.returncode == 0
        assert "Previous release tag: v1.0.0" in result.stdout
        assert "+10000 bytes" in result.stdout or "+9.8 KiB" in result.stdout

    def test_over_budget_still_reports_delta(self, tmp_path):
        """Delta is reported even when contract exceeds budget."""
        git_dir = str(tmp_path / "repo")
        os.makedirs(git_dir)
        _init_git_repo(git_dir)

        wasm_dir = os.path.join(git_dir, "target", "wasm32-unknown-unknown", "release")
        os.makedirs(wasm_dir)
        for contract, size in [
            ("fluxora_stream", 200000),
            ("fluxora_factory", 100000),
            ("fluxora_governance", 100000),
        ]:
            _make_wasm(wasm_dir, f"{contract}.wasm", size)
        _git_commit(git_dir, "baseline")
        _git_tag(git_dir, "v1.0.0")

        # Stream exceeds budget.
        for contract, size in [
            ("fluxora_stream", 262145),
            ("fluxora_factory", 105000),
            ("fluxora_governance", 102000),
        ]:
            _make_wasm(wasm_dir, f"{contract}.wasm", size)
        _git_commit(git_dir, "over budget")

        result = self._invoke(wasm_dir, git_dir)
        assert result.returncode == 1
        # Delta should still be reported even though budget failed.
        assert "vs v1.0.0" in result.stdout

    def test_multiple_tags_uses_latest(self, tmp_path):
        """Delta is computed against the most recent release tag."""
        git_dir = str(tmp_path / "repo")
        os.makedirs(git_dir)
        _init_git_repo(git_dir)

        wasm_dir = os.path.join(git_dir, "target", "wasm32-unknown-unknown", "release")
        os.makedirs(wasm_dir)

        # Create first tag with small WASM.
        for contract, size in [
            ("fluxora_stream", 100000),
            ("fluxora_factory", 50000),
            ("fluxora_governance", 50000),
        ]:
            _make_wasm(wasm_dir, f"{contract}.wasm", size)
        _git_commit(git_dir, "v1")
        _git_tag(git_dir, "v1.0.0")

        # Create second tag with medium WASM.
        for contract, size in [
            ("fluxora_stream", 150000),
            ("fluxora_factory", 75000),
            ("fluxora_governance", 75000),
        ]:
            _make_wasm(wasm_dir, f"{contract}.wasm", size)
        _git_commit(git_dir, "v2")
        _git_tag(git_dir, "v2.0.0")

        # Current build has larger WASM than v2.0.0.
        for contract, size in [
            ("fluxora_stream", 160000),
            ("fluxora_factory", 80000),
            ("fluxora_governance", 80000),
        ]:
            _make_wasm(wasm_dir, f"{contract}.wasm", size)
        _git_commit(git_dir, "current")

        result = self._invoke(wasm_dir, git_dir)
        assert result.returncode == 0
        # Should compare against v2.0.0, not v1.0.0.
        assert "Previous release tag: v2.0.0" in result.stdout
        # Delta should be +10000 from v2.0.0, not +60000 from v1.0.0.
        assert "+10000 bytes" in result.stdout or "+9.8 KiB" in result.stdout

    def test_missing_artifact_with_delta(self, tmp_path):
        """Missing artifact shows error in output and delta for other contracts."""
        git_dir = str(tmp_path / "repo")
        os.makedirs(git_dir)
        _init_git_repo(git_dir)

        wasm_dir = os.path.join(git_dir, "target", "wasm32-unknown-unknown", "release")
        os.makedirs(wasm_dir)
        for contract, size in [
            ("fluxora_stream", 200000),
            ("fluxora_factory", 100000),
            ("fluxora_governance", 100000),
        ]:
            _make_wasm(wasm_dir, f"{contract}.wasm", size)
        _git_commit(git_dir, "baseline")
        _git_tag(git_dir, "v1.0.0")

        # Remove governance WASM.
        os.remove(os.path.join(wasm_dir, "fluxora_governance.wasm"))
        _git_commit(git_dir, "remove governance")

        result = self._invoke(wasm_dir, git_dir)
        assert result.returncode == 1
        # Error should mention the missing artifact.
        assert "not found" in result.stderr or "Artifact not found" in result.stderr
        # Delta for present contracts should still be reported.
        assert "vs v1.0.0" in result.stdout
