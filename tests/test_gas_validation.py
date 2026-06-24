import json
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
