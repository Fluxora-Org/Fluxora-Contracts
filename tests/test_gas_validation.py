import json
import os
import stat
import subprocess
import tempfile
import pytest
from unittest.mock import patch
from script.validate_gas import extract_baselines, parse_measurements, main


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
        mock_baselines.return_value = {"transfer": 2000}
        mock_run_tests.return_value = "GAS_MEASUREMENT: transfer: single: 1900"
        main()
        mock_exit.assert_called_with(0)

    @patch("script.validate_gas.run_tests")
    @patch("script.validate_gas.extract_baselines")
    @patch("script.validate_gas.sys.exit")
    def test_main_with_regression(self, mock_exit, mock_baselines, mock_run_tests):
        """Test failure when gas regression is detected."""
        mock_baselines.return_value = {"transfer": 1000}
        mock_run_tests.return_value = "GAS_MEASUREMENT: transfer: single: 1100"
        main()
        mock_exit.assert_called_with(1)

    @patch("script.validate_gas.run_tests")
    @patch("script.validate_gas.extract_baselines")
    @patch("script.validate_gas.sys.exit")
    def test_main_no_measurements(self, mock_exit, mock_baselines, mock_run_tests):
        """Test error when no measurements found."""
        mock_baselines.return_value = {"transfer": 2000}
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
