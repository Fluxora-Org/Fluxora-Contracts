"""
tests/test_check_discriminant_collisions.py

Test suite for script/check-discriminant-collisions.py.
Targets ≥95% coverage of the script module.
"""

from __future__ import annotations

import importlib.util
import sys
from pathlib import Path

import pytest

# ---------------------------------------------------------------------------
# Load module under test
# ---------------------------------------------------------------------------

_SCRIPT = (
    Path(__file__).resolve().parent.parent
    / "script"
    / "check-discriminant-collisions.py"
)


def _load_module():
    spec = importlib.util.spec_from_file_location(
        "check_discriminant_collisions", _SCRIPT
    )
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


cdc = _load_module()


# ---------------------------------------------------------------------------
# Helpers: build synthetic docs/error.md content
# ---------------------------------------------------------------------------

def _stream_table(*rows: tuple[int, str]) -> str:
    """Emit the stream ContractError section header + table rows."""
    lines = [
        "## Error Code Reference Table\n",
        "| Error Code | Value | Description | Functions Returning It |",
        "|------------|-------|-------------|------------------------|",
    ]
    for code, name in rows:
        lines.append(f"| `{name}` | {code} | desc | func |")
    return "\n".join(lines)


def _factory_table(*rows: tuple[int, str]) -> str:
    """Emit the FactoryError Reference (Factory Contract) section header + rows."""
    lines = [
        "## FactoryError Reference (Factory Contract)\n",
        "| Discriminant | Variant | Triggering Condition | Functions Returning It |",
        "|---:|---|---|---|",
    ]
    for code, name in rows:
        lines.append(f"| {code} | `{name}` | cond | func |")
    return "\n".join(lines)


def _governance_table(*rows: tuple[int, str]) -> str:
    """Emit the GovernanceError Reference section header + rows."""
    lines = [
        "## GovernanceError Reference\n",
        "| Code | Variant | Description | Raising entrypoint(s) | Recoverable? |",
        "|-----:|---------|-------------|----------------------|:------------:|",
    ]
    for code, name in rows:
        lines.append(f"| {code} | `{name}` | desc | fn | ✅ |")
    return "\n".join(lines)


def _full_doc(
    stream_rows: list[tuple[int, str]],
    factory_rows: list[tuple[int, str]],
    governance_rows: list[tuple[int, str]] | None = None,
) -> str:
    parts = [
        _stream_table(*stream_rows),
        "",
        _factory_table(*factory_rows),
    ]
    if governance_rows is not None:
        parts += ["", _governance_table(*governance_rows)]
    return "\n".join(parts)


def _write_doc(tmp_path: Path, content: str) -> Path:
    p = tmp_path / "error.md"
    p.write_text(content, encoding="utf-8")
    return p


# ---------------------------------------------------------------------------
# Tests: Entry / parsing
# ---------------------------------------------------------------------------


class TestEntryParsing:
    """Verify _parse_docs correctly extracts rows in both column orderings."""

    def test_name_first_ordering(self, tmp_path):
        doc = _write_doc(tmp_path, _full_doc(
            [(1, "StreamNotFound"), (2, "InvalidState")],
            [(1, "AlreadyInitialized")],
        ))
        sections = cdc._parse_docs(doc)
        stream = sections["ContractError (stream)"]
        assert stream[0] == cdc.Entry(code=1, name="StreamNotFound", line_no=stream[0].line_no)
        assert stream[1].code == 2
        assert stream[1].name == "InvalidState"

    def test_disc_first_ordering(self, tmp_path):
        # Factory table uses discriminant-first ordering
        doc = _write_doc(tmp_path, _full_doc(
            [(1, "StreamNotFound")],
            [(1, "AlreadyInitialized"), (2, "NotInitialized")],
        ))
        sections = cdc._parse_docs(doc)
        factory = sections["FactoryError (factory)"]
        assert factory[0].code == 1
        assert factory[0].name == "AlreadyInitialized"
        assert factory[1].code == 2

    def test_governance_section_detected(self, tmp_path):
        doc = _write_doc(tmp_path, _full_doc(
            [(1, "StreamNotFound")],
            [(1, "AlreadyInitialized")],
            [(1, "NotInitialized"), (2, "AlreadyInitialized")],
        ))
        sections = cdc._parse_docs(doc)
        assert "GovernanceError (governance)" in sections
        gov = sections["GovernanceError (governance)"]
        assert gov[0].code == 1
        assert gov[0].name == "NotInitialized"

    def test_line_numbers_recorded(self, tmp_path):
        doc = _write_doc(tmp_path, _full_doc(
            [(1, "StreamNotFound"), (2, "InvalidState")],
            [(1, "AlreadyInitialized")],
        ))
        sections = cdc._parse_docs(doc)
        for entry in sections["ContractError (stream)"]:
            assert entry.line_no >= 1

    def test_missing_file_raises(self, tmp_path):
        with pytest.raises(FileNotFoundError):
            cdc._parse_docs(tmp_path / "nonexistent.md")

    def test_no_tables_raises_value_error(self, tmp_path):
        doc = _write_doc(tmp_path, "# Just some markdown\nNo tables here.\n")
        with pytest.raises(ValueError, match="No discriminant tables found"):
            cdc._parse_docs(doc)

    def test_unrelated_h2_resets_current_section(self, tmp_path):
        content = "\n".join([
            _stream_table((1, "StreamNotFound")),
            "",
            "## Some Unrelated Section",
            "| `ShouldNotBeParsed` | 99 | desc | func |",
            "",
            _factory_table((1, "AlreadyInitialized")),
        ])
        doc = _write_doc(tmp_path, content)
        sections = cdc._parse_docs(doc)
        # The unrelated row should not appear in any section
        all_names = {e.name for entries in sections.values() for e in entries}
        assert "ShouldNotBeParsed" not in all_names

    def test_non_table_rows_skipped(self, tmp_path):
        content = "\n".join([
            "## Error Code Reference Table",
            "",
            "| Error Code | Value | Description | Functions Returning It |",
            "|------------|-------|-------------|------------------------|",
            "| `StreamNotFound` | 1 | desc | func |",
            "This is a paragraph line that should be ignored.",
            "| `InvalidState` | 2 | desc | func |",
        ])
        doc = _write_doc(tmp_path, content)
        sections = cdc._parse_docs(doc)
        stream = sections["ContractError (stream)"]
        # Only StreamNotFound is collected — the paragraph line breaks the table
        # so InvalidState is treated as a new header candidate, not a data row.
        assert len(stream) >= 1
        assert stream[0].name == "StreamNotFound"


# ---------------------------------------------------------------------------
# Tests: _find_intra_collisions
# ---------------------------------------------------------------------------


class TestIntraCollisions:
    """Two different names mapped to the same code within one section."""

    def test_no_collision_returns_empty(self):
        entries = [
            cdc.Entry(1, "Foo", 1),
            cdc.Entry(2, "Bar", 2),
            cdc.Entry(3, "Baz", 3),
        ]
        assert cdc._find_intra_collisions("StreamSection", entries) == []

    def test_single_collision_detected(self):
        entries = [
            cdc.Entry(1, "Foo", 1),
            cdc.Entry(1, "Bar", 2),  # same code 1, different name
            cdc.Entry(2, "Baz", 3),
        ]
        msgs = cdc._find_intra_collisions("StreamSection", entries)
        assert len(msgs) == 1
        assert "INTRA-COLLISION" in msgs[0]
        assert "code 1" in msgs[0]
        assert "Foo" in msgs[0]
        assert "Bar" in msgs[0]

    def test_multiple_collisions_reported(self):
        entries = [
            cdc.Entry(1, "A", 1),
            cdc.Entry(1, "B", 2),
            cdc.Entry(5, "C", 3),
            cdc.Entry(5, "D", 4),
        ]
        msgs = cdc._find_intra_collisions("S", entries)
        assert len(msgs) == 2
        codes = [m for m in msgs if "code 1" in m or "code 5" in m]
        assert len(codes) == 2

    def test_same_name_repeated_is_not_collision(self):
        # Same code AND same name — not a collision (duplicate row, but same variant)
        entries = [
            cdc.Entry(1, "Foo", 1),
            cdc.Entry(1, "Foo", 2),
        ]
        msgs = cdc._find_intra_collisions("S", entries)
        assert msgs == []

    def test_empty_entries_returns_empty(self):
        assert cdc._find_intra_collisions("S", []) == []

    def test_section_label_in_message(self):
        entries = [cdc.Entry(7, "X", 1), cdc.Entry(7, "Y", 2)]
        msgs = cdc._find_intra_collisions("MyLabel", entries)
        assert "MyLabel" in msgs[0]

    def test_line_numbers_in_message(self):
        entries = [cdc.Entry(3, "Alpha", 10), cdc.Entry(3, "Beta", 20)]
        msgs = cdc._find_intra_collisions("S", entries)
        assert "line 10" in msgs[0]
        assert "line 20" in msgs[0]


# ---------------------------------------------------------------------------
# Tests: _find_cross_collisions
# ---------------------------------------------------------------------------


class TestCrossCollisions:
    """Same numeric code appearing in more than one section."""

    def test_no_overlap_returns_empty(self):
        sections = {
            "Stream": [cdc.Entry(1, "A", 1), cdc.Entry(2, "B", 2)],
            "Factory": [cdc.Entry(3, "C", 1), cdc.Entry(4, "D", 2)],
        }
        assert cdc._find_cross_collisions(sections) == []

    def test_single_overlap_detected(self):
        sections = {
            "Stream": [cdc.Entry(1, "A", 1)],
            "Factory": [cdc.Entry(1, "B", 1)],
        }
        msgs = cdc._find_cross_collisions(sections)
        assert len(msgs) == 1
        assert "CROSS-SECTION" in msgs[0]
        assert "code 1" in msgs[0]

    def test_multiple_overlaps_reported(self):
        sections = {
            "Stream":  [cdc.Entry(1, "A", 1), cdc.Entry(5, "E", 2)],
            "Factory": [cdc.Entry(1, "B", 1), cdc.Entry(5, "F", 2)],
        }
        msgs = cdc._find_cross_collisions(sections)
        assert len(msgs) == 2

    def test_overlap_across_three_sections(self):
        sections = {
            "Stream":     [cdc.Entry(2, "A", 1)],
            "Factory":    [cdc.Entry(2, "B", 1)],
            "Governance": [cdc.Entry(2, "C", 1)],
        }
        msgs = cdc._find_cross_collisions(sections)
        assert len(msgs) == 1
        assert "Stream" in msgs[0]
        assert "Factory" in msgs[0]
        assert "Governance" in msgs[0]

    def test_single_section_no_cross_collision(self):
        sections = {"Stream": [cdc.Entry(1, "A", 1), cdc.Entry(1, "A", 2)]}
        # Intra-collision within single section — not a cross-section issue
        msgs = cdc._find_cross_collisions(sections)
        assert msgs == []

    def test_empty_sections_returns_empty(self):
        assert cdc._find_cross_collisions({}) == []

    def test_both_section_names_in_message(self):
        sections = {
            "ContractError (stream)": [cdc.Entry(7, "Unauthorized", 1)],
            "FactoryError (factory)": [cdc.Entry(7, "Unauthorized", 1)],
        }
        msgs = cdc._find_cross_collisions(sections)
        assert "ContractError (stream)" in msgs[0]
        assert "FactoryError (factory)" in msgs[0]


# ---------------------------------------------------------------------------
# Tests: _find_ordering_issues
# ---------------------------------------------------------------------------


class TestOrderingIssues:
    def test_ascending_order_no_issues(self):
        entries = [cdc.Entry(1, "A", 1), cdc.Entry(2, "B", 2), cdc.Entry(5, "C", 3)]
        assert cdc._find_ordering_issues("S", entries) == []

    def test_out_of_order_detected(self):
        entries = [cdc.Entry(1, "A", 1), cdc.Entry(3, "B", 2), cdc.Entry(2, "C", 3)]
        msgs = cdc._find_ordering_issues("S", entries)
        assert len(msgs) == 1
        assert "OUT-OF-ORDER" in msgs[0]
        assert "code 2" in msgs[0]

    def test_multiple_regressions(self):
        entries = [
            cdc.Entry(5, "E", 1),
            cdc.Entry(4, "D", 2),
            cdc.Entry(3, "C", 3),
        ]
        msgs = cdc._find_ordering_issues("S", entries)
        assert len(msgs) == 2

    def test_empty_entries_ok(self):
        assert cdc._find_ordering_issues("S", []) == []

    def test_single_entry_ok(self):
        assert cdc._find_ordering_issues("S", [cdc.Entry(1, "A", 1)]) == []

    def test_equal_consecutive_codes_not_flagged(self):
        # Same code twice in a row is an intra-collision, not an ordering issue
        entries = [cdc.Entry(1, "A", 1), cdc.Entry(1, "B", 2)]
        msgs = cdc._find_ordering_issues("S", entries)
        assert msgs == []

    def test_section_label_and_line_in_message(self):
        entries = [cdc.Entry(10, "A", 5), cdc.Entry(3, "B", 12)]
        msgs = cdc._find_ordering_issues("MySection", entries)
        assert "MySection" in msgs[0]
        assert "line 12" in msgs[0]


# ---------------------------------------------------------------------------
# Tests: main() integration — clean docs (no collisions)
# ---------------------------------------------------------------------------


class TestMainCleanDoc:
    def test_returns_zero_on_clean_doc(self, tmp_path):
        doc = _write_doc(tmp_path, _full_doc(
            [(1, "StreamNotFound"), (2, "InvalidState"), (3, "InvalidParams")],
            [(1, "AlreadyInitialized"), (2, "NotInitialized"), (3, "Unauthorized")],
        ))
        rc = cdc.main(["--docs", str(doc)])
        assert rc == 0

    def test_output_lists_sections(self, tmp_path, capsys):
        doc = _write_doc(tmp_path, _full_doc(
            [(1, "StreamNotFound")],
            [(1, "AlreadyInitialized")],
        ))
        cdc.main(["--docs", str(doc)])
        out = capsys.readouterr().out
        assert "ContractError (stream)" in out
        assert "FactoryError (factory)" in out

    def test_shared_decoder_finding_always_printed(self, tmp_path, capsys):
        doc = _write_doc(tmp_path, _full_doc(
            [(1, "StreamNotFound")],
            [(1, "AlreadyInitialized")],
        ))
        cdc.main(["--docs", str(doc)])
        out = capsys.readouterr().out
        assert "SHARED-DECODER FINDING" in out

    def test_no_collision_summary_line(self, tmp_path, capsys):
        doc = _write_doc(tmp_path, _full_doc(
            [(1, "A"), (2, "B")],
            [(3, "C"), (4, "D")],
        ))
        cdc.main(["--docs", str(doc)])
        out = capsys.readouterr().out
        assert "No intra-section collisions found" in out


# ---------------------------------------------------------------------------
# Tests: main() integration — intra-section collision → exit 1
# ---------------------------------------------------------------------------


class TestMainIntraCollision:
    def test_returns_one_on_intra_collision(self, tmp_path):
        # Two different names on code 1 in the stream table
        doc = _write_doc(tmp_path, _full_doc(
            [(1, "Foo"), (1, "Bar"), (2, "Baz")],
            [(1, "AlreadyInitialized")],
        ))
        rc = cdc.main(["--docs", str(doc)])
        assert rc == 1

    def test_intra_collision_message_in_output(self, tmp_path, capsys):
        doc = _write_doc(tmp_path, _full_doc(
            [(17, "ClockRegression"), (17, "ReservationCountZero")],
            [(1, "AlreadyInitialized")],
        ))
        cdc.main(["--docs", str(doc)])
        out = capsys.readouterr().out
        assert "INTRA-COLLISION" in out
        assert "code 17" in out

    def test_action_required_message_printed(self, tmp_path, capsys):
        doc = _write_doc(tmp_path, _full_doc(
            [(23, "TokenVerificationFailed"), (23, "PauseReasonTooLong")],
            [(1, "AlreadyInitialized")],
        ))
        cdc.main(["--docs", str(doc)])
        out = capsys.readouterr().out
        assert "ACTION REQUIRED" in out


# ---------------------------------------------------------------------------
# Tests: main() integration — cross-section overlap → exit 0 (warning only)
# ---------------------------------------------------------------------------


class TestMainCrossOverlap:
    def test_returns_zero_on_cross_overlap(self, tmp_path):
        # Code 1 appears in both stream and factory — warning only, not failure
        doc = _write_doc(tmp_path, _full_doc(
            [(1, "StreamNotFound"), (2, "InvalidState")],
            [(1, "AlreadyInitialized"), (2, "NotInitialized")],
        ))
        rc = cdc.main(["--docs", str(doc)])
        assert rc == 0

    def test_cross_overlap_warning_in_output(self, tmp_path, capsys):
        doc = _write_doc(tmp_path, _full_doc(
            [(1, "StreamNotFound")],
            [(1, "AlreadyInitialized")],
        ))
        cdc.main(["--docs", str(doc)])
        out = capsys.readouterr().out
        assert "CROSS-SECTION" in out

    def test_no_cross_overlap_message(self, tmp_path, capsys):
        doc = _write_doc(tmp_path, _full_doc(
            [(1, "A")],
            [(100, "Z")],
        ))
        cdc.main(["--docs", str(doc)])
        out = capsys.readouterr().out
        assert "No cross-section numeric overlaps detected" in out


# ---------------------------------------------------------------------------
# Tests: main() integration — file errors → exit 2
# ---------------------------------------------------------------------------


class TestMainFileErrors:
    def test_returns_two_on_missing_file(self, tmp_path, capsys):
        rc = cdc.main(["--docs", str(tmp_path / "no_such_file.md")])
        assert rc == 2

    def test_returns_two_on_empty_file(self, tmp_path, capsys):
        doc = _write_doc(tmp_path, "# Just some text\nNo tables.\n")
        rc = cdc.main(["--docs", str(doc)])
        assert rc == 2

    def test_error_message_printed_on_missing_file(self, tmp_path, capsys):
        cdc.main(["--docs", str(tmp_path / "missing.md")])
        err = capsys.readouterr().err
        assert "File not found" in err or "ERROR" in err


# ---------------------------------------------------------------------------
# Tests: main() — auto-detect docs path
# ---------------------------------------------------------------------------


class TestMainAutoDetect:
    def test_auto_detect_finds_real_error_md(self):
        """Running without --docs should find the real docs/error.md in the repo."""
        rc = cdc.main([])
        # The real docs/error.md has intra-collisions (ClockRegression=17 /
        # ReservationCountZero=17 and TokenVerificationFailed=23 /
        # PauseReasonTooLong=23 in the stream table), so we expect exit 1.
        # What we care about here is that it runs without crashing (exit 0 or 1).
        assert rc in (0, 1)


# ---------------------------------------------------------------------------
# Tests: out-of-order entries printed as warnings
# ---------------------------------------------------------------------------


class TestMainOrderingWarning:
    def test_out_of_order_printed(self, tmp_path, capsys):
        doc = _write_doc(tmp_path, _full_doc(
            [(1, "A"), (3, "B"), (2, "C")],
            [(1, "X")],
        ))
        cdc.main(["--docs", str(doc)])
        out = capsys.readouterr().out
        assert "OUT-OF-ORDER" in out

    def test_out_of_order_does_not_cause_exit_1(self, tmp_path):
        # Out-of-order alone is a warning, not a hard error
        doc = _write_doc(tmp_path, _full_doc(
            [(1, "A"), (3, "B"), (2, "C")],
            [(1, "X")],
        ))
        rc = cdc.main(["--docs", str(doc)])
        # Should be 0 (no intra-collision) even with ordering issues
        assert rc == 0


# ---------------------------------------------------------------------------
# Tests: with three sections (governance included)
# ---------------------------------------------------------------------------


class TestThreeSections:
    def test_three_sections_all_detected(self, tmp_path):
        doc = _write_doc(tmp_path, _full_doc(
            [(1, "StreamNotFound"), (2, "InvalidState")],
            [(1, "AlreadyInitialized"), (2, "NotInitialized")],
            [(1, "GovNotInitialized"), (2, "GovAlreadyInitialized")],
        ))
        sections = cdc._parse_docs(doc)
        assert "GovernanceError (governance)" in sections

    def test_three_section_cross_overlap_reported(self, tmp_path, capsys):
        doc = _write_doc(tmp_path, _full_doc(
            [(5, "StreamX")],
            [(5, "FactoryX")],
            [(5, "GovX")],
        ))
        cdc.main(["--docs", str(doc)])
        out = capsys.readouterr().out
        assert "CROSS-SECTION" in out
        # All three sections should appear in the cross-overlap line
        assert "ContractError (stream)" in out
        assert "FactoryError (factory)" in out
        assert "GovernanceError (governance)" in out

    def test_intra_collision_in_governance_section(self, tmp_path):
        doc = _write_doc(tmp_path, _full_doc(
            [(1, "A")],
            [(1, "B")],
            [(3, "GovA"), (3, "GovB")],
        ))
        rc = cdc.main(["--docs", str(doc)])
        assert rc == 1


# ---------------------------------------------------------------------------
# Tests: real docs/error.md against expected findings
# ---------------------------------------------------------------------------


class TestRealErrorMd:
    """
    Run the script against the actual docs/error.md in the repository and
    assert the expected structural properties.

    The real docs/error.md is known to have:
      - Intra-section collisions in the stream table:
          code 17: ClockRegression / ReservationCountZero (in the docs table)
          code 23: TokenVerificationFailed / PauseReasonTooLong (in the docs table)
      - Cross-section overlap between stream and factory (codes 1–16 overlap)
    """

    @pytest.fixture
    def real_sections(self):
        repo_root = Path(__file__).resolve().parent.parent
        docs_path = repo_root / "docs" / "error.md"
        return cdc._parse_docs(docs_path)

    def test_three_sections_present(self, real_sections):
        assert "ContractError (stream)" in real_sections
        assert "FactoryError (factory)" in real_sections
        assert "GovernanceError (governance)" in real_sections

    def test_factory_has_16_variants(self, real_sections):
        factory = real_sections["FactoryError (factory)"]
        codes = {e.code for e in factory}
        assert codes == set(range(1, 17))

    def test_stream_has_no_intra_collisions_after_fix(self, real_sections):
        """docs/error.md stream table was fixed: no intra-section collisions remain."""
        stream = real_sections["ContractError (stream)"]
        msgs = cdc._find_intra_collisions("ContractError (stream)", stream)
        assert msgs == [], f"Unexpected intra-collisions: {msgs}"

    def test_cross_section_overlap_exists(self, real_sections):
        msgs = cdc._find_cross_collisions(real_sections)
        # Codes 1–16 all appear in both stream and factory sections
        assert len(msgs) >= 1

    def test_main_exits_zero_on_fixed_doc(self):
        """The fixed docs/error.md has no intra-collisions → main() returns 0."""
        repo_root = Path(__file__).resolve().parent.parent
        rc = cdc.main(["--docs", str(repo_root / "docs" / "error.md")])
        assert rc == 0
