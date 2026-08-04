"""
tests/test_check_storage_layout_diff.py
========================================

Comprehensive unit and integration tests for
``script/check_storage_layout_diff.py``.

The module under test implements a CI gate that detects breaking ``DataKey``
storage-layout changes between two git refs.  It:

  - Reads ``DataKey`` enums from ``contracts/stream/src/lib.rs`` and
    ``contracts/factory/src/lib.rs`` at both the base and head refs.
  - Compares the variant lists position-by-position (0-based discriminant).
  - Reports renames, removals, and field-shape changes as **breaking**.
  - Allows strictly-additive changes (new variants appended at the end).

Coverage targets
----------------
>=95% line coverage, covering:

  - All ``diff_datakey_variants`` branches (rename, shape change, removal,
    additive append).
  - ``check_file`` error paths (missing file, deleted file, no enum found,
    new file added).
  - ``main()`` end-to-end (real git repo with two commits).
  - ``__name__ == '__main__'`` guard.
  - CLI argument forwarding (--base, --head, --files, --repo-root).

Security guarantees under test
-------------------------------
- A variant rename at any position is always flagged as breaking.
- A variant field-shape change is always flagged as breaking.
- A variant removal is always flagged as breaking.
- A variant inserted at an existing position (pushing later variants) is
  detected as a rename at that position.
- New variants appended after all original variants are silently accepted.
- A file added in head (did not exist in base) is treated as non-breaking.
- A file deleted in head is treated as breaking.
- Missing ``DataKey`` enum in either version is handled gracefully.
"""

from __future__ import annotations

import importlib.util
import os
import runpy
import subprocess
import sys
import tempfile
from pathlib import Path
from unittest.mock import MagicMock, patch, ANY

import pytest

# ---------------------------------------------------------------------------
# Load module under test
# ---------------------------------------------------------------------------

_SCRIPT = Path(__file__).resolve().parent.parent / "script" / "check_storage_layout_diff.py"


def _load_module():
    """Load the module under test and register it in sys.modules."""
    spec = importlib.util.spec_from_file_location(
        "check_storage_layout_diff", str(_SCRIPT)
    )
    mod = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = mod
    spec.loader.exec_module(mod)
    return mod


csld = _load_module()

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------


def _make_variants(*names_and_shapes: str) -> list:
    """Build a list of VariantDef from compact string specs.

    Each spec is like ``Config`` (unit) or ``Stream(u64)`` (payload).
    """
    result = []
    for i, spec in enumerate(names_and_shapes):
        if "(" in spec:
            name, payload_rest = spec.split("(", 1)
            payload = "(" + payload_rest  # restore the opening paren
        else:
            name = spec
            payload = ""
        result.append(csld.VariantDef(name=name, payload=payload, index=i))
    return result


def _assert_result_types(results, expected_breaking: int, expected_info: int = 0):
    """Assert the counts of [BREAKING]/[ERROR] and [INFO] messages."""
    breaking = sum(1 for r in results if r.startswith("[BREAKING]"))
    errors = sum(1 for r in results if r.startswith("[ERROR]"))
    info = sum(1 for r in results if r.startswith("[INFO]"))
    assert (
        breaking + errors == expected_breaking
    ), f"Expected {expected_breaking} breaking/error msgs, got breaking={breaking}, errors={errors} in: {results}"
    assert info == expected_info, f"Expected {expected_info} info msgs, got {info}"


def _run_git(args, cwd):
    """Run a git command in *cwd*."""
    subprocess.run(
        ["git"] + args,
        cwd=cwd,
        check=True,
        capture_output=True,
        env={
            **os.environ,
            "GIT_AUTHOR_NAME": "Test",
            "GIT_AUTHOR_EMAIL": "test@example.com",
            "GIT_COMMITTER_NAME": "Test",
            "GIT_COMMITTER_EMAIL": "test@example.com",
        },
    )


def _write_file(repo_dir, rel_path, content):
    """Write *content* to *rel_path* inside *repo_dir* and git-add it."""
    full_path = os.path.join(repo_dir, rel_path)
    os.makedirs(os.path.dirname(full_path), exist_ok=True)
    with open(full_path, "w", encoding="utf-8") as f:
        f.write(content)
    _run_git(["add", "-A"], cwd=repo_dir)


def _commit(repo_dir, message):
    """Commit staged changes and return the HEAD SHA."""
    _run_git(["commit", "-m", message], cwd=repo_dir)
    return (
        subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=repo_dir)
        .decode("utf-8")
        .strip()
    )


def _init_repo(path):
    """Initialise a git repository in *path*."""
    _run_git(["init", "-q"], cwd=path)
    # Set user config so git doesn't complain
    _run_git(["config", "user.email", "test@example.com"], cwd=path)
    _run_git(["config", "user.name", "Test"], cwd=path)


def _multi_line_enum(*variants: str) -> str:
    """Build a multi-line DataKey enum body."""
    lines = ["pub enum DataKey {"]
    for v in variants:
        lines.append(f"    {v},")
    lines.append("}")
    return "\n".join(lines)


def _commit_file(repo_dir, rel_path, content, message):
    """Write a file, git-add it, commit it, return SHA."""
    _write_file(repo_dir, rel_path, content)
    return _commit(repo_dir, message)


# ===========================================================================
# Tests: parse_datakey
# ===========================================================================


class TestParseDataKey:
    """Verify enum body parsing captures names, payloads, and indices."""

    def test_simple_unit_variants(self):
        body = "    Config,\n    NextStreamId,\n    Stream(u64),\n"
        variants = csld.parse_datakey(body)
        assert len(variants) == 3
        assert variants[0] == csld.VariantDef("Config", "", 0)
        assert variants[1] == csld.VariantDef("NextStreamId", "", 1)
        assert variants[2] == csld.VariantDef("Stream", "(u64)", 2)

    def test_variants_with_payload(self):
        body = "    Stream(u64),\n    RecipientStreams(Address),\n    AutoClaimDestination(u64),\n"
        variants = csld.parse_datakey(body)
        assert len(variants) == 3
        assert variants[0].name == "Stream"
        assert variants[0].payload == "(u64)"

    def test_mixed_unit_and_payload(self):
        body = "    Config,\n    Stream(u64),\n    TotalLiabilities,\n"
        variants = csld.parse_datakey(body)
        assert len(variants) == 3
        assert variants[0].payload == ""
        assert variants[2].payload == ""

    def test_complex_payloads_with_multiple_args(self):
        body = "    RecipientStreamPage(Address, u32),\n    PooledStreamWithdrawn(u64, Address),\n"
        variants = csld.parse_datakey(body)
        assert len(variants) == 2
        assert "Address" in variants[0].payload
        assert "u32" in variants[0].payload

    def test_with_attributes_is_skipped(self):
        body = '    #[contracttype]\n    #[derive(Clone)]\n    Config,\n    Stream(u64),\n'
        variants = csld.parse_datakey(body)
        assert len(variants) == 2

    def test_with_doc_comments(self):
        body = (
            '    /// This is a doc comment.\n'
            '    /// Another line.\n'
            '    Config,\n'
            '    /// Doc for Stream\n'
            '    Stream(u64),\n'
        )
        variants = csld.parse_datakey(body)
        assert len(variants) == 2

    def test_empty_body_raises(self):
        with pytest.raises(ValueError, match="No DataKey variants"):
            csld.parse_datakey("    // just a comment\n")

    def test_inline_comments_after_variant(self):
        body = "    Config, // global settings\n    Stream(u64),\n"
        variants = csld.parse_datakey(body)
        assert len(variants) == 2

    def test_no_trailing_comma(self):
        body = "    Config,\n    Stream(u64)\n    TotalLiabilities\n"
        variants = csld.parse_datakey(body)
        assert len(variants) >= 2

    def test_realistic_stream_body(self):
        """Parse a subset that mirrors the real DataKey body."""
        body = """    Config,
    NextStreamId,
    Stream(u64),
    RecipientStreams(Address),
    GlobalEmergencyPaused,
    CreationPaused,
    GlobalPauseReason,
    GlobalPauseTimestamp,
    GlobalPauseAdmin,
    TotalLiabilities,
    WithdrawNonce(Address),
    PauseState,
"""
        variants = csld.parse_datakey(body)
        assert len(variants) == 12
        assert variants[0].name == "Config"
        assert variants[10].name == "WithdrawNonce"
        assert variants[10].payload == "(Address)"


# ===========================================================================
# Tests: extract_datakey_variants
# ===========================================================================


class TestExtractDataKeyVariants:
    """Verify extraction from full Rust source text."""

    def test_finds_datakey_enum(self):
        source = _multi_line_enum("Config", "Stream(u64)")
        variants = csld.extract_datakey_variants(source)
        assert len(variants) == 2
        assert variants[0].name == "Config"
        assert variants[1].name == "Stream"

    def test_no_datakey_returns_empty(self):
        source = "pub enum Foo { Bar, Baz }"
        variants = csld.extract_datakey_variants(source)
        assert variants == []

    def test_multiple_enums_only_datakey_extracted(self):
        source = f"""
pub enum Foo {{ A, B }}
{_multi_line_enum("Config", "Stream(u64)")}
pub enum Bar {{ X, Y }}
"""
        variants = csld.extract_datakey_variants(source)
        assert len(variants) == 2

    def test_empty_source_returns_empty(self):
        assert csld.extract_datakey_variants("") == []

    def test_datakey_with_inner_attributes(self):
        source = _multi_line_enum(
            "/// The first variant.\n    Config",
            "/// Stream data.\n    Stream(u64)",
            "/// Last variant.\n    TotalLiabilities",
        )
        variants = csld.extract_datakey_variants(source)
        assert len(variants) == 3
        assert variants[-1].name == "TotalLiabilities"


# ===========================================================================
# Tests: diff_datakey_variants
# ===========================================================================


class TestDiffDataKeyVariants:
    """Test the core diff logic."""

    LABEL = "test.rs"

    def test_identical_variants_yields_empty(self):
        base = _make_variants("Config", "Stream(u64)", "TotalLiabilities")
        head = _make_variants("Config", "Stream(u64)", "TotalLiabilities")
        results = csld.diff_datakey_variants(self.LABEL, base, head)
        assert results == []

    def test_append_at_end_is_non_breaking(self):
        base = _make_variants("Config", "Stream(u64)")
        head = _make_variants("Config", "Stream(u64)", "NewVariant", "AnotherVariant(Address)")
        results = csld.diff_datakey_variants(self.LABEL, base, head)
        _assert_result_types(results, expected_breaking=0, expected_info=1)
        assert "NewVariant" in results[0]

    def test_rename_detected(self):
        base = _make_variants("Config", "Stream(u64)", "TotalLiabilities")
        head = _make_variants("Config", "RenamedStream(u64)", "TotalLiabilities")
        results = csld.diff_datakey_variants(self.LABEL, base, head)
        _assert_result_types(results, expected_breaking=1)
        assert "renamed" in results[0]
        assert "index 1" in results[0]

    def test_field_shape_change_detected(self):
        base = _make_variants("Config", "Stream(u64)")
        head = _make_variants("Config", "Stream(Address)")
        results = csld.diff_datakey_variants(self.LABEL, base, head)
        _assert_result_types(results, expected_breaking=1)
        assert "changed shape" in results[0]

    def test_unit_to_payload_is_shape_change(self):
        base = _make_variants("Config", "Stream")
        head = _make_variants("Config", "Stream(u64)")
        results = csld.diff_datakey_variants(self.LABEL, base, head)
        _assert_result_types(results, expected_breaking=1)
        assert "changed shape" in results[0]

    def test_payload_to_unit_is_shape_change(self):
        base = _make_variants("Stream(u64)")
        head = _make_variants("Stream")
        results = csld.diff_datakey_variants(self.LABEL, base, head)
        _assert_result_types(results, expected_breaking=1)

    def test_removal_detected(self):
        base = _make_variants("Config", "Stream(u64)", "TotalLiabilities")
        head = _make_variants("Config", "Stream(u64)")
        results = csld.diff_datakey_variants(self.LABEL, base, head)
        _assert_result_types(results, expected_breaking=1)
        assert "removed" in results[0]

    def test_multiple_removals_detected(self):
        base = _make_variants("A", "B", "C", "D")
        head = _make_variants("A", "B")
        results = csld.diff_datakey_variants(self.LABEL, base, head)
        _assert_result_types(results, expected_breaking=1)
        assert "C" in results[0] and "D" in results[0]

    def test_rename_at_first_position(self):
        base = _make_variants("Config", "Stream(u64)")
        head = _make_variants("RenamedConfig", "Stream(u64)")
        results = csld.diff_datakey_variants(self.LABEL, base, head)
        _assert_result_types(results, expected_breaking=1)
        assert "index 0" in results[0]

    def test_rename_plus_append(self):
        base = _make_variants("Config", "Stream(u64)")
        head = _make_variants("Config", "RenamedStream(u64)", "NewVariant")
        results = csld.diff_datakey_variants(self.LABEL, base, head)
        _assert_result_types(results, expected_breaking=1, expected_info=1)
        breaking = [r for r in results if r.startswith("[BREAKING]")]
        info = [r for r in results if r.startswith("[INFO]")]
        assert "RenamedStream" in breaking[0]

    def test_empty_base_empty_head(self):
        assert csld.diff_datakey_variants("test", [], []) == []

    def test_empty_base_yields_info_only(self):
        base: list = []
        head = _make_variants("Config", "Stream(u64)")
        results = csld.diff_datakey_variants("test", base, head)
        _assert_result_types(results, expected_breaking=0, expected_info=1)

    def test_nonempty_base_empty_head_reports_removal(self):
        base = _make_variants("Config")
        head: list = []
        results = csld.diff_datakey_variants("test", base, head)
        _assert_result_types(results, expected_breaking=1)

    def test_label_appears_in_messages(self):
        base = _make_variants("Config")
        head = _make_variants("Renamed")
        results = csld.diff_datakey_variants("my_custom_path.rs", base, head)
        assert "my_custom_path.rs" in results[0]

    def test_insert_at_middle_detected_as_rename(self):
        """Inserting a variant in the middle shifts later names -> renames."""
        base = _make_variants("A", "C")
        head = _make_variants("A", "B", "C")
        results = csld.diff_datakey_variants("test", base, head)
        # At index 1: base had "C", head has "B" -> rename
        _assert_result_types(results, expected_breaking=1, expected_info=1)
        breaking_msgs = [r for r in results if r.startswith("[BREAKING]")]
        assert any("C" in r or "B" in r for r in breaking_msgs)


# ===========================================================================
# Tests: check_file
# ===========================================================================


class TestCheckFile:
    """Test the file-level check function with mocked content."""

    LABEL = "contracts/stream/src/lib.rs"

    def test_file_added_in_head(self):
        """New file that didn't exist in base is non-breaking."""
        results = csld.check_file(self.LABEL,
                                   base_text=None,
                                   head_text=_multi_line_enum("Config", "Stream(u64)"))
        _assert_result_types(results, expected_breaking=0, expected_info=1)
        assert "new file" in results[0]

    def test_file_deleted_in_head(self):
        """File that existed in base but was deleted in head is breaking."""
        results = csld.check_file(self.LABEL,
                                   base_text=_multi_line_enum("Config"),
                                   head_text=None)
        _assert_result_types(results, expected_breaking=1)
        assert "deleted" in results[0]

    def test_file_missing_at_both_refs(self):
        results = csld.check_file(self.LABEL, base_text=None, head_text=None)
        assert "[SKIP]" in results[0]

    def test_head_has_no_datakey_enum(self):
        results = csld.check_file(self.LABEL,
                                   base_text=_multi_line_enum("Config"),
                                   head_text="pub mod foo;")
        _assert_result_types(results, expected_breaking=1)
        assert "no DataKey enum found" in results[0]

    def test_base_has_no_datakey_but_file_exists(self):
        """File exists but has no DataKey enum in base -> [ERROR] is emitted but
        the change is not treated as breaking since the base didn't have a DataKey."""
        results = csld.check_file(self.LABEL,
                                   base_text="pub mod foo;",
                                   head_text=_multi_line_enum("Config"))
        _assert_result_types(results, expected_breaking=1, expected_info=0)
        assert "no DataKey enum found in base version" in results[0]

    def test_identical_files(self):
        content = _multi_line_enum("Config", "Stream(u64)")
        results = csld.check_file(self.LABEL, base_text=content, head_text=content)
        assert results == []

    def test_breaking_change_in_file(self):
        base = _multi_line_enum("Config", "Stream(u64)")
        head = _multi_line_enum("Config", "Renamed(u64)")
        results = csld.check_file(self.LABEL, base_text=base, head_text=head)
        _assert_result_types(results, expected_breaking=1)

    def test_additive_change_in_file(self):
        base = _multi_line_enum("Config", "Stream(u64)")
        head = _multi_line_enum("Config", "Stream(u64)", "NewVariant")
        results = csld.check_file(self.LABEL, base_text=base, head_text=head)
        _assert_result_types(results, expected_breaking=0, expected_info=1)

    def test_label_in_messages(self):
        base = _multi_line_enum("Config")
        head = _multi_line_enum("Renamed")
        results = csld.check_file("my_label.rs", base, head)
        assert "my_label.rs" in results[0]


# ===========================================================================
# Tests: read_file_from_git
# ===========================================================================


class TestReadFileFromGit:
    """Test git file reading with mocked subprocess."""

    @patch("subprocess.check_output")
    def test_reads_from_git_with_ref(self, mock_co):
        mock_co.return_value = b"pub enum DataKey { Config }"
        content = csld.read_file_from_git("abc123", "contracts/stream/src/lib.rs")
        assert content == "pub enum DataKey { Config }"
        mock_co.assert_called_once_with(
            ["git", "show", "abc123:contracts/stream/src/lib.rs"],
            stderr=subprocess.DEVNULL,
        )

    @patch("subprocess.check_output")
    def test_git_error_returns_none(self, mock_co):
        mock_co.side_effect = subprocess.CalledProcessError(128, "git")
        assert csld.read_file_from_git("HEAD", "missing.rs") is None

    @patch.object(Path, "read_text", return_value="local content")
    @patch.object(Path, "exists", return_value=True)
    def test_reads_from_local_when_ref_none(self, mock_exists, mock_read_text):
        content = csld.read_file_from_git(None, "local.rs")
        assert content == "local content"

    @patch.object(Path, "exists", return_value=False)
    def test_local_file_missing_returns_none(self, mock_exists):
        assert csld.read_file_from_git(None, "absent.rs") is None

    @patch("subprocess.check_output")
    def test_git_unicode_content(self, mock_co):
        payload = "pub enum DataKey { Config }"
        mock_co.return_value = payload.encode("utf-8")
        content = csld.read_file_from_git("HEAD", "f.rs")
        assert content == payload


# ===========================================================================
# Tests: main() — integration
# ===========================================================================


class TestMain:
    """Test main() with various mocked scenarios."""

    # --- No-op: no tracked files changes ---

    @patch("check_storage_layout_diff.TRACKED_PATHS", ["contracts/stream/src/lib.rs"])
    @patch("check_storage_layout_diff.read_file_from_git")
    def test_exits_0_when_files_identical(self, mock_read):
        content = _multi_line_enum("Config", "Stream(u64)")
        mock_read.side_effect = [content, content]
        with patch("sys.argv", ["check_storage_layout_diff.py"]):
            rc = csld.main()
        assert rc == 0

    # --- Breaking change -> exit 1 ---

    @patch("check_storage_layout_diff.TRACKED_PATHS", ["contracts/stream/src/lib.rs"])
    @patch("check_storage_layout_diff.read_file_from_git")
    def test_exits_1_on_breaking_change(self, mock_read):
        mock_read.side_effect = [
            _multi_line_enum("Config", "Stream(u64)"),
            _multi_line_enum("Config", "Renamed(u64)"),
        ]
        with patch("sys.argv", ["check_storage_layout_diff.py"]):
            rc = csld.main()
        assert rc == 1

    # --- Additive change -> exit 0 ---

    @patch("check_storage_layout_diff.TRACKED_PATHS", ["contracts/stream/src/lib.rs"])
    @patch("check_storage_layout_diff.read_file_from_git")
    def test_exits_0_on_additive_change(self, mock_read):
        mock_read.side_effect = [
            _multi_line_enum("Config", "Stream(u64)"),
            _multi_line_enum("Config", "Stream(u64)", "NewVariant(Address)"),
        ]
        with patch("sys.argv", ["check_storage_layout_diff.py"]):
            rc = csld.main()
        assert rc == 0

    # --- New file added -> exit 0 ---

    @patch("check_storage_layout_diff.TRACKED_PATHS", ["contracts/new/src/lib.rs"])
    @patch("check_storage_layout_diff.read_file_from_git")
    def test_exits_0_on_new_file(self, mock_read):
        mock_read.side_effect = [
            None,
            _multi_line_enum("Config"),
        ]
        with patch("sys.argv", ["check_storage_layout_diff.py"]):
            rc = csld.main()
        assert rc == 0

    # --- File deleted -> exit 1 ---

    @patch("check_storage_layout_diff.TRACKED_PATHS", ["contracts/stream/src/lib.rs"])
    @patch("check_storage_layout_diff.read_file_from_git")
    def test_exits_1_on_deleted_file(self, mock_read):
        mock_read.side_effect = [
            _multi_line_enum("Config"),
            None,
        ]
        with patch("sys.argv", ["check_storage_layout_diff.py"]):
            rc = csld.main()
        assert rc == 1

    # --- Multiple files: one breaking, one OK -> exit 1 ---

    @patch("check_storage_layout_diff.TRACKED_PATHS", [
        "contracts/stream/src/lib.rs",
        "contracts/factory/src/lib.rs",
    ])
    @patch("check_storage_layout_diff.read_file_from_git")
    def test_exits_1_when_one_file_breaking(self, mock_read):
        mock_read.side_effect = [
            _multi_line_enum("Config", "Stream(u64)"),
            _multi_line_enum("Config", "Stream(u64)"),
            _multi_line_enum("Admin", "StreamContract"),
            _multi_line_enum("Admin", "Renamed"),
        ]
        with patch("sys.argv", ["check_storage_layout_diff.py"]):
            rc = csld.main()
        assert rc == 1

    # --- CLI args forwarding ---

    @patch("check_storage_layout_diff.TRACKED_PATHS", ["test.rs"])
    @patch("check_storage_layout_diff.read_file_from_git")
    def test_cli_args_base_and_head_forwarded(self, mock_read):
        content = _multi_line_enum("Config")
        mock_read.side_effect = [content, content]
        with patch("sys.argv", [
            "check_storage_layout_diff.py",
            "--base", "origin/main",
            "--head", "feature-branch",
        ]):
            csld.main()
        mock_read.assert_any_call("origin/main", ANY, ANY)
        mock_read.assert_any_call("feature-branch", ANY, ANY)

    @patch("check_storage_layout_diff.TRACKED_PATHS", ["test.rs"])
    @patch("check_storage_layout_diff.read_file_from_git")
    def test_default_cli_args(self, mock_read):
        content = _multi_line_enum("Config")
        mock_read.side_effect = [content, content]
        with patch("sys.argv", ["check_storage_layout_diff.py"]):
            csld.main()
        mock_read.assert_any_call("HEAD", ANY, ANY)
        mock_read.assert_any_call(None, ANY, ANY)

    @patch("check_storage_layout_diff.TRACKED_PATHS", ["custom.rs"])
    @patch("check_storage_layout_diff.read_file_from_git")
    def test_custom_files_arg(self, mock_read):
        content = _multi_line_enum("Config")
        mock_read.side_effect = [content, content]
        with patch("sys.argv", [
            "check_storage_layout_diff.py",
            "--files", "custom.rs",
        ]):
            csld.main()
        mock_read.assert_any_call("HEAD", ANY, ANY)

    @patch("check_storage_layout_diff.read_file_from_git")
    def test_repo_root_arg(self, mock_read):
        mock_read.side_effect = [_multi_line_enum("Config"), None]
        with patch("sys.argv", [
            "check_storage_layout_diff.py",
            "--repo-root", "/tmp/test-repo",
            "--files", "test.rs",
        ]):
            csld.main()
        # The git path should be relative (test.rs), not absolute
        mock_read.assert_any_call("HEAD", "test.rs", "/tmp/test-repo/test.rs")


# ===========================================================================
# Tests: __main__ guard
# ===========================================================================


class TestMainGuard:
    """Verify the ``if __name__ == '__main__'`` guard."""

    def test_main_called_when_run_as_script(self):
        """Prove ``main()`` is called when the module is run directly."""
        result = subprocess.run(
            [sys.executable, str(_SCRIPT), "--base", "HEAD", "--files", "nonexistent.rs"],
            capture_output=True,
            text=True,
        )
        assert "Traceback" not in result.stderr
        assert result.returncode in (0, 1, 2, 128)

    def test_main_not_called_when_imported(self):
        """Prove ``main()`` is NOT called on normal import."""
        with patch.object(csld, "main") as mock_main:
            runpy.run_path(str(_SCRIPT), run_name="check_storage_layout_diff")
        mock_main.assert_not_called()


# ===========================================================================
# Tests: Real git end-to-end
# ===========================================================================


class TestMainEndToEndRealGitRepo:
    """Exercises ``main()`` against real git history with real subprocess calls.

    Each test creates a throwaway git repository, commits a baseline Rust source
    file with a ``DataKey`` enum, commits a mutated version, then invokes
    ``csld.main()`` with the real base and head SHAs.
    """

    REL_PATH = "contracts/stream/src/lib.rs"

    BASE_SOURCE = _multi_line_enum(
        "Config",
        "NextStreamId",
        "Stream(u64)",
        "RecipientStreams(Address)",
        "TotalLiabilities",
    )

    def test_breaking_rename_exits_1(self, monkeypatch):
        with tempfile.TemporaryDirectory() as repo:
            _init_repo(repo)
            base_sha = _commit_file(repo, self.REL_PATH, self.BASE_SOURCE, "base")
            head_source = self.BASE_SOURCE.replace("Stream(u64)", "RenamedStream(u64)")
            head_sha = _commit_file(repo, self.REL_PATH, head_source, "head")

            monkeypatch.chdir(repo)
            rc = csld.main([
                "--base", base_sha,
                "--head", head_sha,
                "--files", self.REL_PATH,
            ])
            assert rc == 1

    def test_breaking_shape_change_exits_1(self, monkeypatch):
        with tempfile.TemporaryDirectory() as repo:
            _init_repo(repo)
            base_sha = _commit_file(repo, self.REL_PATH, self.BASE_SOURCE, "base")
            head_source = self.BASE_SOURCE.replace("Stream(u64)", "Stream(Address)")
            head_sha = _commit_file(repo, self.REL_PATH, head_source, "head")

            monkeypatch.chdir(repo)
            rc = csld.main([
                "--base", base_sha,
                "--head", head_sha,
                "--files", self.REL_PATH,
            ])
            assert rc == 1

    def test_breaking_removal_exits_1(self, monkeypatch):
        with tempfile.TemporaryDirectory() as repo:
            _init_repo(repo)
            base_sha = _commit_file(repo, self.REL_PATH, self.BASE_SOURCE, "base")
            head_source = self.BASE_SOURCE.replace("    TotalLiabilities,\n", "")
            head_sha = _commit_file(repo, self.REL_PATH, head_source, "head")

            monkeypatch.chdir(repo)
            rc = csld.main([
                "--base", base_sha,
                "--head", head_sha,
                "--files", self.REL_PATH,
            ])
            assert rc == 1

    def test_additive_append_exits_0(self, monkeypatch):
        with tempfile.TemporaryDirectory() as repo:
            _init_repo(repo)
            base_sha = _commit_file(repo, self.REL_PATH, self.BASE_SOURCE, "base")
            head_source = self.BASE_SOURCE + "    NewVariant,\n"
            head_sha = _commit_file(repo, self.REL_PATH, head_source, "head")

            monkeypatch.chdir(repo)
            rc = csld.main([
                "--base", base_sha,
                "--head", head_sha,
                "--files", self.REL_PATH,
            ])
            assert rc == 0

    def test_no_change_exits_0(self, monkeypatch):
        with tempfile.TemporaryDirectory() as repo:
            _init_repo(repo)
            base_sha = _commit_file(repo, self.REL_PATH, self.BASE_SOURCE, "base")
            # Same content but force a commit
            _write_file(repo, "OTHER_FILE.md", "unrelated change")
            head_sha = _commit(repo, "head")

            monkeypatch.chdir(repo)
            rc = csld.main([
                "--base", base_sha,
                "--head", head_sha,
                "--files", self.REL_PATH,
            ])
            assert rc == 0

    def test_insert_in_middle_detected_as_breaking(self, monkeypatch):
        with tempfile.TemporaryDirectory() as repo:
            _init_repo(repo)
            base_sha = _commit_file(repo, self.REL_PATH, self.BASE_SOURCE, "base")
            head_source = self.BASE_SOURCE.replace(
                "    RecipientStreams(Address),\n",
                "    MiddleVariant(Address),\n    RecipientStreams(Address),\n",
            )
            head_sha = _commit_file(repo, self.REL_PATH, head_source, "head")

            monkeypatch.chdir(repo)
            rc = csld.main([
                "--base", base_sha,
                "--head", head_sha,
                "--files", self.REL_PATH,
            ])
            assert rc == 1

    def test_factory_and_stream_both_checked(self, monkeypatch):
        with tempfile.TemporaryDirectory() as repo:
            _init_repo(repo)

            stream_rel = "contracts/stream/src/lib.rs"
            factory_rel = "contracts/factory/src/lib.rs"

            stream_source = _multi_line_enum("Config", "Stream(u64)")
            factory_source = _multi_line_enum("Admin", "StreamContract")

            _commit_file(repo, stream_rel, stream_source, "base: stream")
            base_sha = _commit_file(repo, factory_rel, factory_source, "base: factory")

            factory_head = _multi_line_enum("Admin", "Renamed")
            head_sha = _commit_file(repo, factory_rel, factory_head, "head")

            monkeypatch.chdir(repo)
            rc = csld.main([
                "--base", base_sha,
                "--head", head_sha,
                "--files", stream_rel, factory_rel,
            ])
            assert rc == 1


# ===========================================================================
# Cross-version invariants
# ===========================================================================


class TestCrossVersionInvariants:
    """Verify the script can parse the *real* DataKey enums in the repository."""

    def test_can_parse_real_stream_datakey(self):
        repo_root = Path(__file__).resolve().parent.parent
        stream_lib = repo_root / "contracts" / "stream" / "src" / "lib.rs"
        if not stream_lib.exists():
            pytest.skip("Source file not found")
        text = stream_lib.read_text(encoding="utf-8")
        variants = csld.extract_datakey_variants(text)
        assert len(variants) >= 37
        assert variants[0].name == "Config"
        assert variants[1].name == "NextStreamId"
        assert variants[2].name == "Stream"
        assert variants[2].payload == "(u64)"

    def test_can_parse_real_factory_datakey(self):
        repo_root = Path(__file__).resolve().parent.parent
        factory_lib = repo_root / "contracts" / "factory" / "src" / "lib.rs"
        if not factory_lib.exists():
            pytest.skip("Source file not found")
        text = factory_lib.read_text(encoding="utf-8")
        variants = csld.extract_datakey_variants(text)
        assert len(variants) >= 10
        assert variants[0].name == "Admin"
        assert variants[1].name == "StreamContract"
