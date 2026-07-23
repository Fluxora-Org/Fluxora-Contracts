import pytest
import sys
import os
from unittest.mock import patch, MagicMock

# Add script dir to sys.path so we can import check_snapshot_diff
sys.path.append(os.path.join(os.path.dirname(__file__), '..', 'script'))
import check_snapshot_diff

def test_is_security_relevant():
    assert check_snapshot_diff.is_security_relevant("tx.events[0].topic") == True
    assert check_snapshot_diff.is_security_relevant("tx.auths[0].signatures") == True
    assert check_snapshot_diff.is_security_relevant("ContractError.code") == True
    assert check_snapshot_diff.is_security_relevant("data.storage.state") == True
    assert check_snapshot_diff.is_security_relevant("tx.fee") == False
    assert check_snapshot_diff.is_security_relevant("timestamp") == False

def test_get_diff_paths():
    old = {"a": 1, "b": {"c": 2, "d": [1, 2]}}
    new = {"a": 1, "b": {"c": 3, "d": [1, 3]}}
    diffs = check_snapshot_diff.get_diff_paths(old, new)
    assert set(diffs) == {"b.c", "b.d[1]"}

    old_list = [{"id": 1}, {"id": 2}]
    new_list = [{"id": 1}, {"id": 3}]
    diffs_list = check_snapshot_diff.get_diff_paths(old_list, new_list)
    assert set(diffs_list) == {"[1].id"}
    
    diffs_type = check_snapshot_diff.get_diff_paths({"a": 1}, {"a": "1"})
    assert set(diffs_type) == {"a"}

    diffs_len = check_snapshot_diff.get_diff_paths([1, 2], [1])
    assert set(diffs_len) == {""}

    diffs_missing = check_snapshot_diff.get_diff_paths({"a": 1}, {"b": 2})
    assert set(diffs_missing) == {"a", "b"}

@patch('subprocess.check_output')
def test_get_changed_files(mock_check_output):
    mock_check_output.return_value = b"contracts/stream/test_snapshots/a.json\nother.txt\n"
    
    files = check_snapshot_diff.get_changed_files("HEAD~1", "HEAD")
    assert files == ["contracts/stream/test_snapshots/a.json"]
    mock_check_output.assert_called_with(['git', 'diff', '--name-only', 'HEAD~1', 'HEAD'])

    # Test head=None
    files_nohead = check_snapshot_diff.get_changed_files("HEAD~1", None)
    assert files_nohead == ["contracts/stream/test_snapshots/a.json"]

@patch('subprocess.check_output')
def test_get_changed_files_error(mock_check_output):
    import subprocess
    mock_check_output.side_effect = subprocess.CalledProcessError(1, 'git')
    files = check_snapshot_diff.get_changed_files("HEAD", "HEAD")
    assert files == []

@patch('subprocess.check_output')
def test_get_file_content_git(mock_check_output):
    mock_check_output.return_value = b'{"test": 1}'
    content = check_snapshot_diff.get_file_content("HEAD", "file.json")
    assert content == '{"test": 1}'

@patch('subprocess.check_output')
def test_get_file_content_git_error(mock_check_output):
    import subprocess
    mock_check_output.side_effect = subprocess.CalledProcessError(1, 'git')
    content = check_snapshot_diff.get_file_content("HEAD", "file.json")
    assert content is None

@patch('os.path.exists')
@patch('builtins.open', new_callable=MagicMock)
def test_get_file_content_local(mock_open, mock_exists):
    mock_exists.return_value = True
    mock_open.return_value.__enter__.return_value.read.return_value = '{"test": 1}'
    content = check_snapshot_diff.get_file_content(None, "file.json")
    assert content == '{"test": 1}'

@patch('os.path.exists')
def test_get_file_content_local_not_exists(mock_exists):
    mock_exists.return_value = False
    content = check_snapshot_diff.get_file_content(None, "file.json")
    assert content is None

@patch('check_snapshot_diff.get_changed_files')
@patch('check_snapshot_diff.get_file_content')
def test_main_no_files(mock_get_file_content, mock_get_changed_files, capsys):
    mock_get_changed_files.return_value = []
    with patch('sys.argv', ['check_snapshot_diff.py']), pytest.raises(SystemExit) as e:
        check_snapshot_diff.main()
    assert e.value.code == 0
    captured = capsys.readouterr()
    assert "No snapshot JSON files changed." in captured.out

@patch('check_snapshot_diff.get_changed_files')
@patch('check_snapshot_diff.get_file_content')
def test_main_security_diff(mock_get_file_content, mock_get_changed_files, capsys):
    mock_get_changed_files.return_value = ["test.json"]
    mock_get_file_content.side_effect = ['{"events": [{"topic": "A"}]}', '{"events": [{"topic": "B"}]}']
    
    with patch('sys.argv', ['check_snapshot_diff.py']), pytest.raises(SystemExit) as e:
        check_snapshot_diff.main()
    
    assert e.value.code == 1
    captured = capsys.readouterr()
    assert "Security-relevant fields changed" in captured.out

@patch('check_snapshot_diff.get_changed_files')
@patch('check_snapshot_diff.get_file_content')
def test_main_no_security_diff(mock_get_file_content, mock_get_changed_files, capsys):
    mock_get_changed_files.return_value = ["test.json"]
    mock_get_file_content.side_effect = ['{"fee": 10}', '{"fee": 20}']
    
    with patch('sys.argv', ['check_snapshot_diff.py']), pytest.raises(SystemExit) as e:
        check_snapshot_diff.main()
    
    assert e.value.code == 0
    captured = capsys.readouterr()
    assert "Changes in test.json (none are security-relevant)" in captured.out

@patch('check_snapshot_diff.get_changed_files')
@patch('check_snapshot_diff.get_file_content')
def test_main_invalid_json(mock_get_file_content, mock_get_changed_files, capsys):
    mock_get_changed_files.return_value = ["test.json"]
    mock_get_file_content.side_effect = ['{invalid}', '{"fee": 20}']
    
    with patch('sys.argv', ['check_snapshot_diff.py']), pytest.raises(SystemExit) as e:
        check_snapshot_diff.main()
    
    assert e.value.code == 0
