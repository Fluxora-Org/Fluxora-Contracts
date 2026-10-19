"""Guards tying docs/KNOWN-LIMITATIONS.md §5 to the executable pieces.

§5 records that the ledger close time the TTL conversion assumes is measured,
not assumed (#1806), with a 20% safety margin covering drift. These tests keep
the documented story tied to the script and source that back it.
"""

import subprocess
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent

MEASURE_SCRIPT = REPO_ROOT / "script" / "measure-ledger-close.sh"
LIMITATIONS = REPO_ROOT / "docs" / "KNOWN-LIMITATIONS.md"
LEDGER_DOC = REPO_ROOT / "docs" / "ledger-close-time.md"
STORAGE_RS = REPO_ROOT / "contracts" / "stream" / "src" / "storage.rs"
TTL_RS = REPO_ROOT / "contracts" / "stream" / "src" / "test" / "ttl.rs"


class TestMeasurementScript:
    """The measurement script exists and is structurally sound."""

    def test_script_exists(self):
        assert MEASURE_SCRIPT.exists(), f"{MEASURE_SCRIPT} not found"

    def test_script_is_valid_bash(self):
        result = subprocess.run(
            ["bash", "-n", str(MEASURE_SCRIPT)], capture_output=True, text=True
        )
        assert result.returncode == 0, f"Bash syntax check failed: {result.stderr}"

    def test_script_reads_constants_from_source(self):
        """The script must parse the constants, not carry its own copy."""
        text = MEASURE_SCRIPT.read_text(encoding="utf-8")
        assert "SECONDS_PER_LEDGER" in text
        assert "TTL_SAFETY_MARGIN_PERCENT" in text
        assert "--verify" in text, "script must support a --verify gate"


class TestLedgerCloseTimeDoc:
    """docs/ledger-close-time.md carries the measurement record."""

    def test_doc_exists(self):
        assert LEDGER_DOC.exists(), f"{LEDGER_DOC} not found"

    def test_doc_records_window_and_mean(self):
        text = LEDGER_DOC.read_text(encoding="utf-8")
        assert "120,960" in text, "measurement window missing"
        assert "5.000" in text, "measured mean missing"


class TestStorageConstantsMatchDocs:
    """The Rust constants and the pinned docs must tell the same story."""

    def _constant(self, name: str) -> int:
        text = STORAGE_RS.read_text(encoding="utf-8")
        needle = f"pub const {name}: u64 = "
        start = text.index(needle) + len(needle)
        digits = ""
        for ch in text[start:]:
            if ch.isdigit():
                digits += ch
            elif digits:
                break
        return int(digits)

    def test_seconds_per_ledger_matches_measured_doc(self):
        # MARKER: observed_mean_seconds (mirrored in test/ttl.rs)
        assert self._constant("SECONDS_PER_LEDGER") == 5
        assert "5.000" in LEDGER_DOC.read_text(encoding="utf-8")

    def test_margin_is_at_least_twenty_percent(self):
        assert self._constant("TTL_SAFETY_MARGIN_PERCENT") >= 20

    def test_ttl_test_pins_the_measured_value(self):
        text = TTL_RS.read_text(encoding="utf-8")
        assert "seconds_per_ledger_matches_the_measured_close_time" in text
        assert "MARKER: observed_mean_seconds" in text


class TestKnownLimitationsSection5:
    """§5 records the measurement instead of the old 'assumed' story."""

    def test_section5_records_the_measurement(self):
        text = LIMITATIONS.read_text(encoding="utf-8")
        assert "## 5. Ledger close time is measured" in text
        assert "docs/ledger-close-time.md" in text
        assert "TTL_SAFETY_MARGIN_PERCENT" in text

    def test_old_assumed_headline_is_gone(self):
        text = LIMITATIONS.read_text(encoding="utf-8")
        assert "## 5. Ledger close time is assumed, not measured" not in text
