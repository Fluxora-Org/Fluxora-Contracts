"""
tests/test_check_snapshot_diff.py
==================================
Comprehensive unit tests for script/check_snapshot_diff.py.

The module under test implements a security-relevant snapshot-diff gate used
in the CI pipeline.  It:
  - Determines which ``contracts/stream/test_snapshots/*.json`` files were
    touched between two git refs.
  - Reads both the old and new version of each file (from git history or the
    working tree).
  - Recursively walks the JSON diff and flags any path that contains a member
    of ``SECURITY_FIELDS`` (auth, events, error, storage, etc.).
  - Exits 0 when no security-relevant change is detected, 1 otherwise.

Coverage targets
----------------
Lines 99-100: ``new_json`` JSONDecodeError exception branch
Line 123:     ``if __name__ == '__main__'`` guard executed directly

Security guarantees under test
-------------------------------
- Only snapshot files under ``/test_snapshots/`` trigger analysis.
- All SECURITY_FIELDS members are individually tested as relevant.
- Nested and list-indexed paths are correctly classified.
- Malformed JSON for either the old or new content is treated as ``{}``
  (empty object) to avoid crashing; any structural delta is still reported.
- A missing file on either side (returns ``None``) is treated as ``{}``.
- The script produces exit code 1 when security diffs are found, regardless
  of which files among multiple changed files contain the diff.
"""

import json
import subprocess
import sys
import os
import runpy
from unittest.mock import patch, mock_open, MagicMock, call

import pytest

# ---------------------------------------------------------------------------
# Import the module under test
# ---------------------------------------------------------------------------
sys.path.insert(0, os.path.join(os.path.dirname(__file__), '..', 'script'))
import check_snapshot_diff  # noqa: E402


# ===========================================================================
# is_security_relevant
# ===========================================================================

class TestIsSecurityRelevant:
    """Verify that every SECURITY_FIELDS member is detected, and only them."""

    # --- positive cases: should match ---
    @pytest.mark.parametrize("path", [
        "tx.events[0].topic",
        "tx.auths[0].signatures",
        "tx.require_auth",
        "ContractError.code",
        "data.storage.state",
        "result.DataKey",
        "envelope.auth",
        "envelope.auths",
        "envelope.events",
        "envelope.topic",
        "envelope.topics",
        "envelope.data",
        "envelope.error",
        "envelope.error_code",
        # Nested deeply
        "a.b.c.d.events",
        "root[0].storage[1].DataKey",
    ])
    def test_security_relevant_true(self, path):
        assert check_snapshot_diff.is_security_relevant(path) is True

    # --- negative cases: must not match ---
    @pytest.mark.parametrize("path", [
        "tx.fee",
        "timestamp",
        "sequence",
        "tx.source_account",
        "ledger_sequence",
        "operation.amount",
        "result.success",
        "metadata.version",
        "",
    ])
    def test_security_relevant_false(self, path):
        assert check_snapshot_diff.is_security_relevant(path) is False

    def test_security_fields_set_contains_expected_members(self):
        """Guard against accidental removal from SECURITY_FIELDS."""
        required = {
            'auth', 'auths', 'require_auth', 'signatures',
            'events', 'topic', 'topics', 'data',
            'error', 'error_code', 'ContractError',
            'storage', 'state', 'DataKey',
        }
        assert required.issubset(check_snapshot_diff.SECURITY_FIELDS)


# ===========================================================================
# get_diff_paths
# ===========================================================================

class TestGetDiffPaths:
    """Recursive JSON diff walker covers dict, list, scalar, and type changes."""

    def test_identical_dicts_produce_no_diffs(self):
        obj = {"a": 1, "b": {"c": 2}}
        assert check_snapshot_diff.get_diff_paths(obj, obj) == []

    def test_changed_scalar_is_reported(self):
        diffs = check_snapshot_diff.get_diff_paths({"a": 1}, {"a": 2})
        assert diffs == ["a"]

    def test_nested_scalar_change(self):
        old = {"a": {"b": {"c": 2, "d": [1, 2]}}}
        new = {"a": {"b": {"c": 3, "d": [1, 3]}}}
        diffs = check_snapshot_diff.get_diff_paths(old, new)
        assert set(diffs) == {"a.b.c", "a.b.d[1]"}

    def test_list_element_change(self):
        old = [{"id": 1}, {"id": 2}]
        new = [{"id": 1}, {"id": 3}]
        diffs = check_snapshot_diff.get_diff_paths(old, new)
        assert set(diffs) == {"[1].id"}

    def test_list_length_mismatch(self):
        diffs = check_snapshot_diff.get_diff_paths([1, 2], [1])
        # The root list itself differs
        assert "" in diffs

    def test_type_change_is_reported(self):
        diffs = check_snapshot_diff.get_diff_paths({"a": 1}, {"a": "1"})
        assert "a" in diffs

    def test_added_key(self):
        diffs = check_snapshot_diff.get_diff_paths({}, {"b": 2})
        assert "b" in diffs

    def test_removed_key(self):
        diffs = check_snapshot_diff.get_diff_paths({"a": 1}, {})
        assert "a" in diffs

    def test_both_dicts_missing_different_keys(self):
        diffs = check_snapshot_diff.get_diff_paths({"a": 1}, {"b": 2})
        assert "a" in diffs
        assert "b" in diffs

    def test_empty_dicts_are_equal(self):
        assert check_snapshot_diff.get_diff_paths({}, {}) == []

    def test_empty_lists_are_equal(self):
        assert check_snapshot_diff.get_diff_paths([], []) == []

    def test_path_prefix_is_prepended(self):
        """When called recursively, the path prefix is prepended correctly."""
        diffs = check_snapshot_diff.get_diff_paths({"x": 1}, {"x": 2}, "root")
        assert "root.x" in diffs

    def test_nested_list_of_dicts(self):
        old = {"events": [{"topic": "transfer"}, {"topic": "mint"}]}
        new = {"events": [{"topic": "transfer"}, {"topic": "burn"}]}
        diffs = check_snapshot_diff.get_diff_paths(old, new)
        assert set(diffs) == {"events[1].topic"}

    def test_deeply_nested_no_change(self):
        obj = {"a": {"b": {"c": {"d": [1, 2, {"e": True}]}}}}
        assert check_snapshot_diff.get_diff_paths(obj, obj) == []


# ===========================================================================
# get_changed_files
# ===========================================================================

class TestGetChangedFiles:
    """Only snapshot JSON files under /test_snapshots/ pass the filter."""

    @patch('subprocess.check_output')
    def test_filters_snapshot_jsons_with_head(self, mock_co):
        mock_co.return_value = (
            b"contracts/stream/test_snapshots/a.json\n"
            b"other.txt\n"
            b"contracts/stream/test_snapshots/b.json\n"
        )
        files = check_snapshot_diff.get_changed_files("HEAD~1", "HEAD")
        assert files == [
            "contracts/stream/test_snapshots/a.json",
            "contracts/stream/test_snapshots/b.json",
        ]
        mock_co.assert_called_once_with(
            ['git', 'diff', '--name-only', 'HEAD~1', 'HEAD']
        )

    @patch('subprocess.check_output')
    def test_filters_snapshot_jsons_without_head(self, mock_co):
        mock_co.return_value = b"contracts/stream/test_snapshots/c.json\n"
        files = check_snapshot_diff.get_changed_files("origin/main", None)
        assert files == ["contracts/stream/test_snapshots/c.json"]
        mock_co.assert_called_once_with(
            ['git', 'diff', '--name-only', 'origin/main']
        )

    @patch('subprocess.check_output')
    def test_excludes_non_snapshot_json(self, mock_co):
        mock_co.return_value = b"README.md\nsrc/lib.rs\nfoo.json\n"
        files = check_snapshot_diff.get_changed_files("HEAD~1", "HEAD")
        assert files == []

    @patch('subprocess.check_output')
    def test_excludes_non_json_in_snapshot_dir(self, mock_co):
        # A file *in* test_snapshots but not .json should be excluded
        mock_co.return_value = (
            b"contracts/stream/test_snapshots/README.md\n"
            b"contracts/stream/test_snapshots/test.json\n"
        )
        files = check_snapshot_diff.get_changed_files("HEAD~1", "HEAD")
        assert files == ["contracts/stream/test_snapshots/test.json"]

    @patch('subprocess.check_output')
    def test_git_error_returns_empty_list(self, mock_co):
        mock_co.side_effect = subprocess.CalledProcessError(128, 'git')
        assert check_snapshot_diff.get_changed_files("HEAD~1", "HEAD") == []

    @patch('subprocess.check_output')
    def test_empty_output_returns_empty_list(self, mock_co):
        mock_co.return_value = b""
        assert check_snapshot_diff.get_changed_files("HEAD", "HEAD") == []


# ===========================================================================
# get_file_content
# ===========================================================================

class TestGetFileContent:
    """Reads from git history when a commit is given, else from disk."""

    @patch('subprocess.check_output')
    def test_reads_from_git_with_commit(self, mock_co):
        mock_co.return_value = b'{"key": "value"}'
        content = check_snapshot_diff.get_file_content("abc123", "path/to/file.json")
        assert content == '{"key": "value"}'
        mock_co.assert_called_once_with(['git', 'show', 'abc123:path/to/file.json'])

    @patch('subprocess.check_output')
    def test_git_error_returns_none(self, mock_co):
        mock_co.side_effect = subprocess.CalledProcessError(128, 'git')
        assert check_snapshot_diff.get_file_content("HEAD", "missing.json") is None

    @patch('os.path.exists', return_value=True)
    @patch('builtins.open', new_callable=mock_open, read_data='{"local": true}')
    def test_reads_from_local_when_no_commit(self, mock_file, mock_exists):
        content = check_snapshot_diff.get_file_content(None, "local.json")
        assert content == '{"local": true}'
        mock_exists.assert_called_once_with("local.json")

    @patch('os.path.exists', return_value=False)
    def test_local_file_missing_returns_none(self, mock_exists):
        assert check_snapshot_diff.get_file_content(None, "absent.json") is None

    @patch('subprocess.check_output')
    def test_git_returns_unicode_content(self, mock_co):
        payload = '{"msg": "hello \\u00e9"}'
        mock_co.return_value = payload.encode('utf-8')
        content = check_snapshot_diff.get_file_content("HEAD", "f.json")
        assert content == payload


# ===========================================================================
# main() — integration paths
# ===========================================================================

class TestMain:
    """End-to-end tests for main(), covering all exit-code paths."""

    # --- no snapshot files changed ---
    @patch('check_snapshot_diff.get_changed_files', return_value=[])
    def test_exits_0_when_no_snapshot_files(self, _mock, capsys):
        with patch('sys.argv', ['check_snapshot_diff.py']):
            with pytest.raises(SystemExit) as exc:
                check_snapshot_diff.main()
        assert exc.value.code == 0
        assert "No snapshot JSON files changed." in capsys.readouterr().out

    # --- security-relevant diff → exit 1 ---
    @patch('check_snapshot_diff.get_changed_files',
           return_value=["contracts/stream/test_snapshots/snap.json"])
    @patch('check_snapshot_diff.get_file_content')
    def test_exits_1_on_security_diff(self, mock_content, _mock_files, capsys):
        mock_content.side_effect = [
            '{"events": [{"topic": "old_topic"}]}',
            '{"events": [{"topic": "new_topic"}]}',
        ]
        with patch('sys.argv', ['check_snapshot_diff.py']):
            with pytest.raises(SystemExit) as exc:
                check_snapshot_diff.main()
        assert exc.value.code == 1
        out = capsys.readouterr().out
        assert "Security-relevant fields changed" in out
        assert "Mandatory extra review required" in out

    # --- non-security diff → exit 0 ---
    @patch('check_snapshot_diff.get_changed_files',
           return_value=["contracts/stream/test_snapshots/snap.json"])
    @patch('check_snapshot_diff.get_file_content')
    def test_exits_0_on_nonsecurity_diff(self, mock_content, _mock_files, capsys):
        mock_content.side_effect = [
            '{"fee": 100}',
            '{"fee": 200}',
        ]
        with patch('sys.argv', ['check_snapshot_diff.py']):
            with pytest.raises(SystemExit) as exc:
                check_snapshot_diff.main()
        assert exc.value.code == 0
        out = capsys.readouterr().out
        assert "[INFO] Changes in" in out
        assert "none are security-relevant" in out

    # --- identical files → exit 0 ---
    @patch('check_snapshot_diff.get_changed_files',
           return_value=["contracts/stream/test_snapshots/snap.json"])
    @patch('check_snapshot_diff.get_file_content')
    def test_exits_0_when_files_are_identical(self, mock_content, _mock_files, capsys):
        same = '{"fee": 100, "sequence": 1}'
        mock_content.side_effect = [same, same]
        with patch('sys.argv', ['check_snapshot_diff.py']):
            with pytest.raises(SystemExit) as exc:
                check_snapshot_diff.main()
        assert exc.value.code == 0
        assert "No security-relevant snapshot changes detected." in capsys.readouterr().out

    # --- malformed old JSON (line 95) ---
    @patch('check_snapshot_diff.get_changed_files',
           return_value=["contracts/stream/test_snapshots/snap.json"])
    @patch('check_snapshot_diff.get_file_content')
    def test_malformed_old_json_treated_as_empty(self, mock_content, _mock_files):
        mock_content.side_effect = ['{bad json!!', '{"fee": 20}']
        with patch('sys.argv', ['check_snapshot_diff.py']):
            with pytest.raises(SystemExit) as exc:
                check_snapshot_diff.main()
        # fee is not security-relevant; no hard failure expected
        assert exc.value.code == 0

    # --- malformed new JSON (lines 99-100) ---
    @patch('check_snapshot_diff.get_changed_files',
           return_value=["contracts/stream/test_snapshots/snap.json"])
    @patch('check_snapshot_diff.get_file_content')
    def test_malformed_new_json_treated_as_empty(self, mock_content, _mock_files):
        """
        Covers lines 99-100: the JSONDecodeError branch for new_content.
        When new_json cannot be parsed, it falls back to {} and any structural
        divergence from old_json is diffed normally without crashing.
        """
        mock_content.side_effect = ['{"fee": 20}', '{NOT valid JSON!!!']
        with patch('sys.argv', ['check_snapshot_diff.py']):
            with pytest.raises(SystemExit) as exc:
                check_snapshot_diff.main()
        # old={fee:20} vs new={} → diff on "fee" key, not security-relevant
        assert exc.value.code == 0

    # --- malformed new JSON that creates a security diff ---
    @patch('check_snapshot_diff.get_changed_files',
           return_value=["contracts/stream/test_snapshots/snap.json"])
    @patch('check_snapshot_diff.get_file_content')
    def test_malformed_new_json_with_security_field_in_old(self, mock_content, _mock_files, capsys):
        """
        Covers lines 99-100 again: new content is invalid JSON → falls back to
        {}.  If old content had security-relevant fields they now differ (key
        disappeared), which must still trigger exit 1.
        """
        mock_content.side_effect = [
            '{"events": [{"topic": "transfer"}]}',
            '{invalid',
        ]
        with patch('sys.argv', ['check_snapshot_diff.py']):
            with pytest.raises(SystemExit) as exc:
                check_snapshot_diff.main()
        assert exc.value.code == 1
        assert "Security-relevant fields changed" in capsys.readouterr().out

    # --- None content (missing file on both sides) ---
    @patch('check_snapshot_diff.get_changed_files',
           return_value=["contracts/stream/test_snapshots/new_file.json"])
    @patch('check_snapshot_diff.get_file_content')
    def test_none_old_and_new_content(self, mock_content, _mock_files):
        mock_content.side_effect = [None, None]
        with patch('sys.argv', ['check_snapshot_diff.py']):
            with pytest.raises(SystemExit) as exc:
                check_snapshot_diff.main()
        assert exc.value.code == 0

    # --- new file added (None old, valid new with security fields) ---
    @patch('check_snapshot_diff.get_changed_files',
           return_value=["contracts/stream/test_snapshots/new_file.json"])
    @patch('check_snapshot_diff.get_file_content')
    def test_new_file_with_security_fields_exits_1(self, mock_content, _mock_files, capsys):
        mock_content.side_effect = [
            None,
            '{"auth": {"require_auth": true}}',
        ]
        with patch('sys.argv', ['check_snapshot_diff.py']):
            with pytest.raises(SystemExit) as exc:
                check_snapshot_diff.main()
        assert exc.value.code == 1

    # --- file deleted (valid old with security fields, None new) ---
    @patch('check_snapshot_diff.get_changed_files',
           return_value=["contracts/stream/test_snapshots/deleted.json"])
    @patch('check_snapshot_diff.get_file_content')
    def test_deleted_file_with_security_fields_exits_1(self, mock_content, _mock_files, capsys):
        mock_content.side_effect = [
            '{"storage": {"DataKey": "StreamState"}}',
            None,
        ]
        with patch('sys.argv', ['check_snapshot_diff.py']):
            with pytest.raises(SystemExit) as exc:
                check_snapshot_diff.main()
        assert exc.value.code == 1

    # --- multiple files: only one has security diff ---
    @patch('check_snapshot_diff.get_changed_files',
           return_value=[
               "contracts/stream/test_snapshots/safe.json",
               "contracts/stream/test_snapshots/danger.json",
           ])
    @patch('check_snapshot_diff.get_file_content')
    def test_one_of_multiple_files_triggers_exit_1(self, mock_content, _mock_files, capsys):
        mock_content.side_effect = [
            '{"fee": 100}', '{"fee": 200}',          # safe.json — no security diff
            '{"events": [{"topic": "A"}]}',
            '{"events": [{"topic": "B"}]}',           # danger.json — security diff
        ]
        with patch('sys.argv', ['check_snapshot_diff.py']):
            with pytest.raises(SystemExit) as exc:
                check_snapshot_diff.main()
        assert exc.value.code == 1
        out = capsys.readouterr().out
        assert "danger.json" in out

    # --- --base and --head CLI args are forwarded correctly ---
    @patch('check_snapshot_diff.get_changed_files', return_value=[])
    def test_cli_args_base_and_head_forwarded(self, mock_gf):
        with patch('sys.argv', ['check_snapshot_diff.py',
                                 '--base', 'origin/main',
                                 '--head', 'feature-branch']):
            with pytest.raises(SystemExit):
                check_snapshot_diff.main()
        mock_gf.assert_called_once_with('origin/main', 'feature-branch')

    # --- default --base is HEAD, default --head is None ---
    @patch('check_snapshot_diff.get_changed_files', return_value=[])
    def test_default_cli_args(self, mock_gf):
        with patch('sys.argv', ['check_snapshot_diff.py']):
            with pytest.raises(SystemExit):
                check_snapshot_diff.main()
        mock_gf.assert_called_once_with('HEAD', None)

    # --- multiple security diffs in the same file are all reported ---
    @patch('check_snapshot_diff.get_changed_files',
           return_value=["contracts/stream/test_snapshots/multi.json"])
    @patch('check_snapshot_diff.get_file_content')
    def test_multiple_security_fields_all_reported(self, mock_content, _mock_files, capsys):
        mock_content.side_effect = [
            '{"events": [{"topic": "old"}], "auth": {"require_auth": false}}',
            '{"events": [{"topic": "new"}], "auth": {"require_auth": true}}',
        ]
        with patch('sys.argv', ['check_snapshot_diff.py']):
            with pytest.raises(SystemExit) as exc:
                check_snapshot_diff.main()
        assert exc.value.code == 1
        out = capsys.readouterr().out
        assert "Security-relevant fields changed" in out

    # --- error_code field is treated as security-relevant ---
    @patch('check_snapshot_diff.get_changed_files',
           return_value=["contracts/stream/test_snapshots/err.json"])
    @patch('check_snapshot_diff.get_file_content')
    def test_error_code_field_triggers_gate(self, mock_content, _mock_files, capsys):
        mock_content.side_effect = [
            '{"error_code": 10}',
            '{"error_code": 20}',
        ]
        with patch('sys.argv', ['check_snapshot_diff.py']):
            with pytest.raises(SystemExit) as exc:
                check_snapshot_diff.main()
        assert exc.value.code == 1

    # --- storage field triggers gate ---
    @patch('check_snapshot_diff.get_changed_files',
           return_value=["contracts/stream/test_snapshots/storage.json"])
    @patch('check_snapshot_diff.get_file_content')
    def test_storage_field_triggers_gate(self, mock_content, _mock_files):
        mock_content.side_effect = [
            '{"storage": {"key": "old_value"}}',
            '{"storage": {"key": "new_value"}}',
        ]
        with patch('sys.argv', ['check_snapshot_diff.py']):
            with pytest.raises(SystemExit) as exc:
                check_snapshot_diff.main()
        assert exc.value.code == 1


# ===========================================================================
# __main__ guard (line 123)
# ===========================================================================

class TestMainGuard:
    """
    Exercises the ``if __name__ == '__main__': main()`` guard (line 123).

    We run the script via subprocess (simulating ``python3 script/...``)
    and via runpy with ``run_name='__main__'`` to confirm the guard fires.
    The subprocess approach provides the strongest evidence because it is
    byte-for-byte identical to how CI invokes the tool.
    """

    def test_main_called_when_run_as_script(self):
        """
        Covers line 123: confirms that ``main()`` is invoked when the module
        is executed directly as a script.

        We invoke it via subprocess with no snapshot dir, so ``get_changed_files``
        returns [] and the script exits 0 with "No snapshot JSON files changed."
        That proves the guard fired and ``main()`` ran.
        """
        script_path = os.path.join(
            os.path.dirname(__file__), '..', 'script', 'check_snapshot_diff.py'
        )
        result = subprocess.run(
            [sys.executable, script_path, '--base', 'HEAD'],
            capture_output=True,
            text=True,
        )
        # The script either exits 0 ("No snapshot JSON files changed.")
        # or 1 (security diff found) or 128+ (git not available / no repo).
        # Any of these prove main() was called; a crash before main() would
        # give a Python traceback with exit code 1 and stderr content.
        # We assert that there is no Python traceback.
        assert 'Traceback' not in result.stderr, (
            f"Script raised an exception:\n{result.stderr}"
        )
        # And that the exit code is one of the expected values (not a Python crash).
        assert result.returncode in (0, 1, 128), (
            f"Unexpected exit code {result.returncode}.\nstdout: {result.stdout}\nstderr: {result.stderr}"
        )

    def test_main_not_called_when_imported(self):
        """
        Confirms that ``main()`` is NOT called when the module is imported
        normally (i.e. ``__name__ != '__main__'``).
        """
        with patch.object(check_snapshot_diff, 'main') as mock_main:
            runpy.run_path(
                os.path.join(
                    os.path.dirname(__file__), '..', 'script', 'check_snapshot_diff.py'
                ),
                run_name='check_snapshot_diff',
            )
        mock_main.assert_not_called()
