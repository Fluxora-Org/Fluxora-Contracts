#!/usr/bin/env python3
"""
tests/test_check_snapshot_diff.py

Comprehensive test suite for script/check_snapshot_diff.py.

Covers:
  - SECURITY_FIELDS registry completeness
  - is_security_relevant() path-matching algorithm (exact, substring, case)
  - flatten_snapshot() recursive flattening (dicts, lists, nesting, scalars)
  - compute_diff() added / removed / changed detection
  - find_security_relevant_changes() classification and sorting
  - format_human() and format_json_output() output formatting
  - load_snapshot() error handling (missing file, bad JSON, wrong type)
  - main() / CLI: exit-code contract (0, 1, 2), --quiet, --output-format json
  - End-to-end integration scenarios drawn from realistic snapshot shapes
"""

from __future__ import annotations

import json
import sys
import os
import tempfile
from pathlib import Path
from unittest.mock import patch, mock_open, MagicMock

import pytest

# ---------------------------------------------------------------------------
# Make sure the repo root is on the path so `script` is importable.
# ---------------------------------------------------------------------------

REPO_ROOT = Path(__file__).resolve().parent.parent
if str(REPO_ROOT) not in sys.path:
    sys.path.insert(0, str(REPO_ROOT))

from script.check_snapshot_diff import (
    SECURITY_FIELDS,
    is_security_relevant,
    flatten_snapshot,
    compute_diff,
    find_security_relevant_changes,
    format_human,
    format_json_output,
    load_snapshot,
    main,
    build_parser,
)


# ---------------------------------------------------------------------------
# Shared snapshot fixtures
# ---------------------------------------------------------------------------

BASE_SNAPSHOT: dict = {
    "config": {
        "admin": "GADMIN111",
        "token": "GTOKEN111",
        "max_rate_per_second": 1000,
    },
    "streams": {
        "0": {
            "sender": "GSENDER1",
            "recipient": "GRECIPIENT1",
            "rate_per_second": 100,
            "deposit_amount": 5000,
            "status": "Active",
            "start_time": 1700000000,
        }
    },
    "next_stream_id": 1,
    "global_emergency_paused": False,
    "creation_paused": False,
}

HEAD_SNAPSHOT_NO_SECURITY: dict = {
    "config": {
        "admin": "GADMIN111",
        "token": "GTOKEN111",
        "max_rate_per_second": 1000,
    },
    "streams": {
        "0": {
            "sender": "GSENDER1",
            "recipient": "GRECIPIENT1",
            "rate_per_second": 100,
            "deposit_amount": 5000,
            "status": "Completed",          # changed — not security-relevant
            "start_time": 1700000000,
        }
    },
    "next_stream_id": 2,                    # changed — not security-relevant
    "global_emergency_paused": False,
    "creation_paused": False,
}

HEAD_SNAPSHOT_ADMIN_CHANGED: dict = {
    "config": {
        "admin": "GADMIN999",               # changed — security-relevant
        "token": "GTOKEN111",
        "max_rate_per_second": 1000,
    },
    "streams": {
        "0": {
            "sender": "GSENDER1",
            "recipient": "GRECIPIENT1",
            "rate_per_second": 100,
            "deposit_amount": 5000,
            "status": "Active",
            "start_time": 1700000000,
        }
    },
    "next_stream_id": 1,
    "global_emergency_paused": False,
    "creation_paused": False,
}


# ---------------------------------------------------------------------------
# Helper: write a snapshot to a temp file and return its Path
# ---------------------------------------------------------------------------

def _write_snapshot(data: dict, suffix: str = ".json") -> Path:
    tmp = tempfile.NamedTemporaryFile(
        mode="w", suffix=suffix, delete=False, encoding="utf-8"
    )
    json.dump(data, tmp)
    tmp.close()
    return Path(tmp.name)


# ---------------------------------------------------------------------------
# 1. SECURITY_FIELDS registry
# ---------------------------------------------------------------------------

class TestSecurityFields:
    """SECURITY_FIELDS must cover all required sentinel categories."""

    def test_is_frozenset(self):
        assert isinstance(SECURITY_FIELDS, frozenset)

    def test_non_empty(self):
        assert len(SECURITY_FIELDS) > 0

    def test_all_lowercase(self):
        for entry in SECURITY_FIELDS:
            assert entry == entry.lower(), (
                f"SECURITY_FIELDS entry '{entry}' is not all-lowercase"
            )

    def test_admin_present(self):
        assert "admin" in SECURITY_FIELDS

    def test_token_present(self):
        assert "token" in SECURITY_FIELDS

    def test_rate_per_second_present(self):
        assert "rate_per_second" in SECURITY_FIELDS

    def test_max_rate_per_second_present(self):
        assert "max_rate_per_second" in SECURITY_FIELDS

    def test_deposit_amount_present(self):
        assert "deposit_amount" in SECURITY_FIELDS

    def test_recipient_present(self):
        assert "recipient" in SECURITY_FIELDS

    def test_paused_present(self):
        assert "paused" in SECURITY_FIELDS

    def test_emergency_present(self):
        assert "emergency" in SECURITY_FIELDS

    def test_nonce_present(self):
        assert "nonce" in SECURITY_FIELDS

    def test_contract_version_present(self):
        assert "contract_version" in SECURITY_FIELDS


# ---------------------------------------------------------------------------
# 2. is_security_relevant
# ---------------------------------------------------------------------------

class TestIsSecurityRelevant:
    """is_security_relevant() exact-match, substring, and negative cases."""

    # --- exact matches ---

    def test_exact_admin(self):
        assert is_security_relevant("admin") is True

    def test_exact_token(self):
        assert is_security_relevant("token") is True

    def test_exact_paused(self):
        assert is_security_relevant("paused") is True

    def test_exact_recipient(self):
        assert is_security_relevant("recipient") is True

    def test_exact_nonce(self):
        assert is_security_relevant("nonce") is True

    def test_exact_emergency(self):
        assert is_security_relevant("emergency") is True

    def test_exact_deposit_amount(self):
        assert is_security_relevant("deposit_amount") is True

    def test_exact_rate_per_second(self):
        assert is_security_relevant("rate_per_second") is True

    def test_exact_max_rate_per_second(self):
        assert is_security_relevant("max_rate_per_second") is True

    def test_exact_contract_version(self):
        assert is_security_relevant("contract_version") is True

    # --- substring / dotted-path matches ---

    def test_dotted_config_admin(self):
        assert is_security_relevant("config.admin") is True

    def test_dotted_config_token(self):
        assert is_security_relevant("config.token") is True

    def test_dotted_streams_rate(self):
        assert is_security_relevant("streams.0.rate_per_second") is True

    def test_dotted_streams_deposit(self):
        assert is_security_relevant("streams.0.deposit_amount") is True

    def test_dotted_streams_recipient(self):
        assert is_security_relevant("streams.0.recipient") is True

    def test_global_emergency_paused(self):
        assert is_security_relevant("global_emergency_paused") is True

    def test_creation_paused(self):
        assert is_security_relevant("creation_paused") is True

    def test_pending_recipient_update(self):
        assert is_security_relevant("pending_recipient_update") is True

    def test_delegated_nonce(self):
        assert is_security_relevant("delegated_nonce") is True

    def test_token_address(self):
        assert is_security_relevant("token_address") is True

    def test_token_contract(self):
        assert is_security_relevant("token_contract") is True

    def test_admin_address(self):
        assert is_security_relevant("admin_address") is True

    # --- case-insensitivity ---

    def test_uppercase_admin(self):
        assert is_security_relevant("ADMIN") is True

    def test_mixed_case_token(self):
        assert is_security_relevant("Token_Address") is True

    def test_uppercase_paused(self):
        assert is_security_relevant("PAUSED") is True

    def test_mixed_case_dotted_path(self):
        assert is_security_relevant("Config.MAX_RATE_PER_SECOND") is True

    # --- negative cases (non-security fields) ---

    def test_status_not_flagged(self):
        assert is_security_relevant("status") is False

    def test_start_time_not_flagged(self):
        assert is_security_relevant("start_time") is False

    def test_end_time_not_flagged(self):
        assert is_security_relevant("end_time") is False

    def test_sender_not_flagged(self):
        assert is_security_relevant("sender") is False

    def test_next_stream_id_not_flagged(self):
        assert is_security_relevant("next_stream_id") is False

    def test_withdrawn_amount_not_flagged(self):
        assert is_security_relevant("withdrawn_amount") is False

    def test_empty_string_not_flagged(self):
        assert is_security_relevant("") is False

    def test_whitespace_not_flagged(self):
        assert is_security_relevant("   ") is False

    def test_cliff_time_not_flagged(self):
        assert is_security_relevant("cliff_time") is False

    def test_stream_id_not_flagged(self):
        assert is_security_relevant("stream_id") is False

    def test_cancelled_at_not_flagged(self):
        assert is_security_relevant("cancelled_at") is False


# ---------------------------------------------------------------------------
# 3. flatten_snapshot
# ---------------------------------------------------------------------------

class TestFlattenSnapshot:
    """flatten_snapshot() must produce correct dotted-key→value mappings."""

    def test_flat_dict(self):
        snap = {"a": 1, "b": 2}
        result = flatten_snapshot(snap)
        assert result == {"a": 1, "b": 2}

    def test_nested_dict(self):
        snap = {"config": {"admin": "G1", "token": "G2"}}
        result = flatten_snapshot(snap)
        assert result == {"config.admin": "G1", "config.token": "G2"}

    def test_deeply_nested(self):
        snap = {"a": {"b": {"c": {"d": 42}}}}
        result = flatten_snapshot(snap)
        assert result == {"a.b.c.d": 42}

    def test_list_values(self):
        snap = {"items": [10, 20, 30]}
        result = flatten_snapshot(snap)
        assert result == {"items.0": 10, "items.1": 20, "items.2": 30}

    def test_dict_inside_list(self):
        snap = {"streams": [{"rate": 5, "status": "Active"}]}
        result = flatten_snapshot(snap)
        assert result == {"streams.0.rate": 5, "streams.0.status": "Active"}

    def test_mixed_nesting(self):
        snap = {"config": {"admin": "G1"}, "ids": [1, 2]}
        result = flatten_snapshot(snap)
        assert result == {
            "config.admin": "G1",
            "ids.0": 1,
            "ids.1": 2,
        }

    def test_empty_dict(self):
        result = flatten_snapshot({})
        assert result == {}

    def test_empty_list_value(self):
        snap = {"items": []}
        result = flatten_snapshot(snap)
        assert result == {}

    def test_empty_nested_dict(self):
        snap = {"config": {}}
        result = flatten_snapshot(snap)
        assert result == {}

    def test_boolean_values(self):
        snap = {"paused": True, "active": False}
        result = flatten_snapshot(snap)
        assert result == {"paused": True, "active": False}

    def test_null_value(self):
        snap = {"field": None}
        result = flatten_snapshot(snap)
        assert result == {"field": None}

    def test_string_value(self):
        snap = {"admin": "GADMIN123"}
        result = flatten_snapshot(snap)
        assert result == {"admin": "GADMIN123"}

    def test_numeric_string_key(self):
        snap = {"streams": {"0": {"rate": 100}}}
        result = flatten_snapshot(snap)
        assert result == {"streams.0.rate": 100}

    def test_list_of_lists(self):
        snap = {"matrix": [[1, 2], [3, 4]]}
        result = flatten_snapshot(snap)
        assert result == {
            "matrix.0.0": 1,
            "matrix.0.1": 2,
            "matrix.1.0": 3,
            "matrix.1.1": 4,
        }


# ---------------------------------------------------------------------------
# 4. compute_diff
# ---------------------------------------------------------------------------

class TestComputeDiff:
    """compute_diff() must correctly categorise added/removed/changed keys."""

    def test_identical_snapshots_produce_empty_diff(self):
        diff = compute_diff(BASE_SNAPSHOT, BASE_SNAPSHOT)
        assert diff["added"] == {}
        assert diff["removed"] == {}
        assert diff["changed"] == {}

    def test_added_key_detected(self):
        head = {**BASE_SNAPSHOT, "new_field": "hello"}
        diff = compute_diff(BASE_SNAPSHOT, head)
        assert "new_field" in diff["added"]
        assert diff["added"]["new_field"] == "hello"

    def test_removed_key_detected(self):
        base = {**BASE_SNAPSHOT, "extra": "bye"}
        diff = compute_diff(base, BASE_SNAPSHOT)
        assert "extra" in diff["removed"]

    def test_changed_value_detected(self):
        diff = compute_diff(BASE_SNAPSHOT, HEAD_SNAPSHOT_ADMIN_CHANGED)
        assert "config.admin" in diff["changed"]
        assert diff["changed"]["config.admin"]["base"] == "GADMIN111"
        assert diff["changed"]["config.admin"]["head"] == "GADMIN999"

    def test_no_security_diff_produces_no_changed_security(self):
        diff = compute_diff(BASE_SNAPSHOT, HEAD_SNAPSHOT_NO_SECURITY)
        # status changed but no security field changed
        assert "streams.0.status" in diff["changed"]
        assert "config.admin" not in diff["changed"]

    def test_empty_base_all_added(self):
        diff = compute_diff({}, {"x": 1})
        assert diff["added"] == {"x": 1}
        assert diff["removed"] == {}
        assert diff["changed"] == {}

    def test_empty_head_all_removed(self):
        diff = compute_diff({"x": 1}, {})
        assert diff["removed"] == {"x": 1}
        assert diff["added"] == {}
        assert diff["changed"] == {}

    def test_both_empty(self):
        diff = compute_diff({}, {})
        assert diff == {"added": {}, "removed": {}, "changed": {}}

    def test_nested_change_detected(self):
        head = json.loads(json.dumps(BASE_SNAPSHOT))
        head["config"]["max_rate_per_second"] = 9999
        diff = compute_diff(BASE_SNAPSHOT, head)
        assert "config.max_rate_per_second" in diff["changed"]

    def test_changed_stores_both_values(self):
        head = json.loads(json.dumps(BASE_SNAPSHOT))
        head["streams"]["0"]["rate_per_second"] = 777
        diff = compute_diff(BASE_SNAPSHOT, head)
        rec = diff["changed"]["streams.0.rate_per_second"]
        assert rec["base"] == 100
        assert rec["head"] == 777

    def test_boolean_flip_detected(self):
        head = json.loads(json.dumps(BASE_SNAPSHOT))
        head["global_emergency_paused"] = True
        diff = compute_diff(BASE_SNAPSHOT, head)
        assert "global_emergency_paused" in diff["changed"]

    def test_none_to_value_is_changed(self):
        base = {"field": None}
        head = {"field": "value"}
        diff = compute_diff(base, head)
        assert "field" in diff["changed"]


# ---------------------------------------------------------------------------
# 5. find_security_relevant_changes
# ---------------------------------------------------------------------------

class TestFindSecurityRelevantChanges:
    """find_security_relevant_changes() must classify and sort correctly."""

    def test_no_changes_returns_empty(self):
        diff = compute_diff(BASE_SNAPSHOT, BASE_SNAPSHOT)
        hits = find_security_relevant_changes(diff)
        assert hits == []

    def test_admin_change_flagged(self):
        diff = compute_diff(BASE_SNAPSHOT, HEAD_SNAPSHOT_ADMIN_CHANGED)
        hits = find_security_relevant_changes(diff)
        keys = [h["key"] for h in hits]
        assert "config.admin" in keys

    def test_non_security_change_not_flagged(self):
        diff = compute_diff(BASE_SNAPSHOT, HEAD_SNAPSHOT_NO_SECURITY)
        hits = find_security_relevant_changes(diff)
        keys = [h["key"] for h in hits]
        assert "streams.0.status" not in keys
        assert "next_stream_id" not in keys

    def test_added_security_field_flagged(self):
        head = json.loads(json.dumps(BASE_SNAPSHOT))
        head["new_admin"] = "GADMINX"
        diff = compute_diff(BASE_SNAPSHOT, head)
        hits = find_security_relevant_changes(diff)
        keys = [h["key"] for h in hits]
        assert "new_admin" in keys

    def test_removed_token_flagged(self):
        base = json.loads(json.dumps(BASE_SNAPSHOT))
        base["config"]["token"] = "GTOKEN_OLD"
        head = json.loads(json.dumps(BASE_SNAPSHOT))
        del head["config"]["token"]
        diff = compute_diff(base, head)
        hits = find_security_relevant_changes(diff)
        keys = [h["key"] for h in hits]
        assert "config.token" in keys

    def test_change_type_field_correct(self):
        diff = compute_diff(BASE_SNAPSHOT, HEAD_SNAPSHOT_ADMIN_CHANGED)
        hits = find_security_relevant_changes(diff)
        admin_hit = next(h for h in hits if h["key"] == "config.admin")
        assert admin_hit["change_type"] == "changed"
        assert admin_hit["base"] == "GADMIN111"
        assert admin_hit["head"] == "GADMIN999"

    def test_added_change_type(self):
        head = json.loads(json.dumps(BASE_SNAPSHOT))
        head["token_address"] = "GNEWTOKEN"
        diff = compute_diff(BASE_SNAPSHOT, head)
        hits = find_security_relevant_changes(diff)
        hit = next(h for h in hits if h["key"] == "token_address")
        assert hit["change_type"] == "added"
        assert hit["base"] is None
        assert hit["head"] == "GNEWTOKEN"

    def test_removed_change_type(self):
        base = json.loads(json.dumps(BASE_SNAPSHOT))
        base["token_address"] = "GTOKEN_OLD"
        diff = compute_diff(base, BASE_SNAPSHOT)
        hits = find_security_relevant_changes(diff)
        hit = next(h for h in hits if h["key"] == "token_address")
        assert hit["change_type"] == "removed"
        assert hit["head"] is None

    def test_results_sorted_by_key(self):
        head = json.loads(json.dumps(BASE_SNAPSHOT))
        head["config"]["admin"] = "GADMIN999"
        head["config"]["max_rate_per_second"] = 9999
        head["global_emergency_paused"] = True
        diff = compute_diff(BASE_SNAPSHOT, head)
        hits = find_security_relevant_changes(diff)
        keys = [h["key"] for h in hits]
        assert keys == sorted(keys)

    def test_multiple_security_changes_all_returned(self):
        head = json.loads(json.dumps(BASE_SNAPSHOT))
        head["config"]["admin"] = "GADMIN999"
        head["config"]["token"] = "GTOKEN999"
        head["global_emergency_paused"] = True
        diff = compute_diff(BASE_SNAPSHOT, head)
        hits = find_security_relevant_changes(diff)
        keys = [h["key"] for h in hits]
        assert "config.admin" in keys
        assert "config.token" in keys
        assert "global_emergency_paused" in keys

    def test_rate_change_flagged(self):
        head = json.loads(json.dumps(BASE_SNAPSHOT))
        head["streams"]["0"]["rate_per_second"] = 9999
        diff = compute_diff(BASE_SNAPSHOT, head)
        hits = find_security_relevant_changes(diff)
        keys = [h["key"] for h in hits]
        assert "streams.0.rate_per_second" in keys

    def test_deposit_amount_change_flagged(self):
        head = json.loads(json.dumps(BASE_SNAPSHOT))
        head["streams"]["0"]["deposit_amount"] = 99999
        diff = compute_diff(BASE_SNAPSHOT, head)
        hits = find_security_relevant_changes(diff)
        keys = [h["key"] for h in hits]
        assert "streams.0.deposit_amount" in keys

    def test_recipient_change_flagged(self):
        head = json.loads(json.dumps(BASE_SNAPSHOT))
        head["streams"]["0"]["recipient"] = "GNEWRECIPIENT"
        diff = compute_diff(BASE_SNAPSHOT, head)
        hits = find_security_relevant_changes(diff)
        keys = [h["key"] for h in hits]
        assert "streams.0.recipient" in keys


# ---------------------------------------------------------------------------
# 6. format_human
# ---------------------------------------------------------------------------

class TestFormatHuman:
    """format_human() output shape for zero, one, and many hits."""

    def test_no_hits_clean_message(self):
        out = format_human([], "base.json", "head.json")
        assert "no security-relevant changes" in out

    def test_one_hit_reports_count(self):
        hits = [{"key": "config.admin", "change_type": "changed",
                 "base": "OLD", "head": "NEW"}]
        out = format_human(hits, "base.json", "head.json")
        assert "1 security-relevant change" in out

    def test_two_hits_reports_count(self):
        hits = [
            {"key": "config.admin", "change_type": "changed", "base": "A", "head": "B"},
            {"key": "config.token", "change_type": "changed", "base": "T1", "head": "T2"},
        ]
        out = format_human(hits, "base.json", "head.json")
        assert "2 security-relevant change" in out

    def test_paths_included(self):
        hits = [{"key": "config.admin", "change_type": "changed",
                 "base": "OLD", "head": "NEW"}]
        out = format_human(hits, "path/to/base.json", "path/to/head.json")
        assert "path/to/base.json" in out
        assert "path/to/head.json" in out

    def test_changed_hit_shows_arrow(self):
        hits = [{"key": "config.admin", "change_type": "changed",
                 "base": "OLD", "head": "NEW"}]
        out = format_human(hits, "b.json", "h.json")
        assert "→" in out
        assert "config.admin" in out

    def test_added_hit_shows_added_tag(self):
        hits = [{"key": "token_address", "change_type": "added",
                 "base": None, "head": "GNEW"}]
        out = format_human(hits, "b.json", "h.json")
        assert "ADDED" in out

    def test_removed_hit_shows_removed_tag(self):
        hits = [{"key": "token_address", "change_type": "removed",
                 "base": "GOLD", "head": None}]
        out = format_human(hits, "b.json", "h.json")
        assert "REMOVED" in out

    def test_mandatory_review_notice_present(self):
        hits = [{"key": "config.admin", "change_type": "changed",
                 "base": "A", "head": "B"}]
        out = format_human(hits, "b.json", "h.json")
        assert "Mandatory extra review" in out or "mandatory extra review" in out.lower()

    def test_doc_link_present(self):
        hits = [{"key": "config.admin", "change_type": "changed",
                 "base": "A", "head": "B"}]
        out = format_human(hits, "b.json", "h.json")
        assert "snapshot-security-diff.md" in out


# ---------------------------------------------------------------------------
# 7. format_json_output
# ---------------------------------------------------------------------------

class TestFormatJsonOutput:
    """format_json_output() must produce valid, structured JSON."""

    def test_valid_json(self):
        out = format_json_output([], "b.json", "h.json", flagged=False)
        parsed = json.loads(out)
        assert isinstance(parsed, dict)

    def test_flagged_false_when_no_hits(self):
        out = format_json_output([], "b.json", "h.json", flagged=False)
        assert json.loads(out)["flagged"] is False

    def test_flagged_true_when_hits(self):
        hits = [{"key": "config.admin", "change_type": "changed",
                 "base": "A", "head": "B"}]
        out = format_json_output(hits, "b.json", "h.json", flagged=True)
        assert json.loads(out)["flagged"] is True

    def test_base_and_head_paths_in_output(self):
        out = format_json_output([], "path/base.json", "path/head.json", flagged=False)
        parsed = json.loads(out)
        assert parsed["base"] == "path/base.json"
        assert parsed["head"] == "path/head.json"

    def test_changes_list_in_output(self):
        hits = [{"key": "config.admin", "change_type": "changed",
                 "base": "A", "head": "B"}]
        out = format_json_output(hits, "b.json", "h.json", flagged=True)
        parsed = json.loads(out)
        assert "security_relevant_changes" in parsed
        assert len(parsed["security_relevant_changes"]) == 1

    def test_empty_changes_list(self):
        out = format_json_output([], "b.json", "h.json", flagged=False)
        parsed = json.loads(out)
        assert parsed["security_relevant_changes"] == []


# ---------------------------------------------------------------------------
# 8. load_snapshot — error handling
# ---------------------------------------------------------------------------

class TestLoadSnapshot:
    """load_snapshot() must exit(2) on bad input."""

    def test_loads_valid_file(self, tmp_path):
        p = tmp_path / "snap.json"
        p.write_text(json.dumps({"admin": "G1"}), encoding="utf-8")
        result = load_snapshot(p)
        assert result == {"admin": "G1"}

    def test_missing_file_exits_2(self, tmp_path):
        with pytest.raises(SystemExit) as exc_info:
            load_snapshot(tmp_path / "nonexistent.json")
        assert exc_info.value.code == 2

    def test_invalid_json_exits_2(self, tmp_path):
        p = tmp_path / "bad.json"
        p.write_text("{not valid json", encoding="utf-8")
        with pytest.raises(SystemExit) as exc_info:
            load_snapshot(p)
        assert exc_info.value.code == 2

    def test_json_array_top_level_exits_2(self, tmp_path):
        p = tmp_path / "arr.json"
        p.write_text("[1, 2, 3]", encoding="utf-8")
        with pytest.raises(SystemExit) as exc_info:
            load_snapshot(p)
        assert exc_info.value.code == 2

    def test_json_string_top_level_exits_2(self, tmp_path):
        p = tmp_path / "str.json"
        p.write_text('"just a string"', encoding="utf-8")
        with pytest.raises(SystemExit) as exc_info:
            load_snapshot(p)
        assert exc_info.value.code == 2

    def test_json_number_top_level_exits_2(self, tmp_path):
        p = tmp_path / "num.json"
        p.write_text("42", encoding="utf-8")
        with pytest.raises(SystemExit) as exc_info:
            load_snapshot(p)
        assert exc_info.value.code == 2

    def test_empty_file_exits_2(self, tmp_path):
        p = tmp_path / "empty.json"
        p.write_text("", encoding="utf-8")
        with pytest.raises(SystemExit) as exc_info:
            load_snapshot(p)
        assert exc_info.value.code == 2

    def test_empty_object_valid(self, tmp_path):
        p = tmp_path / "empty_obj.json"
        p.write_text("{}", encoding="utf-8")
        result = load_snapshot(p)
        assert result == {}


# ---------------------------------------------------------------------------
# 9. main() — exit-code contract and CLI flags
# ---------------------------------------------------------------------------

class TestMain:
    """main() must honour the 0/1/2 exit-code contract and CLI flags."""

    # --- exit 0: no security-relevant changes ---

    def test_exit_0_no_security_changes(self, tmp_path, capsys):
        base_p = _write_snapshot(BASE_SNAPSHOT)
        head_p = _write_snapshot(HEAD_SNAPSHOT_NO_SECURITY)
        try:
            code = main(["--base", str(base_p), "--head", str(head_p)])
            assert code == 0
        finally:
            base_p.unlink(missing_ok=True)
            head_p.unlink(missing_ok=True)

    def test_exit_0_identical_snapshots(self, tmp_path):
        base_p = _write_snapshot(BASE_SNAPSHOT)
        head_p = _write_snapshot(BASE_SNAPSHOT)
        try:
            code = main(["--base", str(base_p), "--head", str(head_p)])
            assert code == 0
        finally:
            base_p.unlink(missing_ok=True)
            head_p.unlink(missing_ok=True)

    # --- exit 1: security-relevant changes present ---

    def test_exit_1_admin_changed(self):
        base_p = _write_snapshot(BASE_SNAPSHOT)
        head_p = _write_snapshot(HEAD_SNAPSHOT_ADMIN_CHANGED)
        try:
            code = main(["--base", str(base_p), "--head", str(head_p)])
            assert code == 1
        finally:
            base_p.unlink(missing_ok=True)
            head_p.unlink(missing_ok=True)

    def test_exit_1_rate_changed(self):
        head = json.loads(json.dumps(BASE_SNAPSHOT))
        head["streams"]["0"]["rate_per_second"] = 9999
        base_p = _write_snapshot(BASE_SNAPSHOT)
        head_p = _write_snapshot(head)
        try:
            code = main(["--base", str(base_p), "--head", str(head_p)])
            assert code == 1
        finally:
            base_p.unlink(missing_ok=True)
            head_p.unlink(missing_ok=True)

    def test_exit_1_token_changed(self):
        head = json.loads(json.dumps(BASE_SNAPSHOT))
        head["config"]["token"] = "GTOKEN_NEW"
        base_p = _write_snapshot(BASE_SNAPSHOT)
        head_p = _write_snapshot(head)
        try:
            code = main(["--base", str(base_p), "--head", str(head_p)])
            assert code == 1
        finally:
            base_p.unlink(missing_ok=True)
            head_p.unlink(missing_ok=True)

    def test_exit_1_emergency_pause_enabled(self):
        head = json.loads(json.dumps(BASE_SNAPSHOT))
        head["global_emergency_paused"] = True
        base_p = _write_snapshot(BASE_SNAPSHOT)
        head_p = _write_snapshot(head)
        try:
            code = main(["--base", str(base_p), "--head", str(head_p)])
            assert code == 1
        finally:
            base_p.unlink(missing_ok=True)
            head_p.unlink(missing_ok=True)

    # --- exit 2: usage errors ---

    def test_exit_2_missing_base_file(self, tmp_path):
        head_p = _write_snapshot(BASE_SNAPSHOT)
        try:
            with pytest.raises(SystemExit) as exc_info:
                main(["--base", str(tmp_path / "missing.json"),
                      "--head", str(head_p)])
            assert exc_info.value.code == 2
        finally:
            head_p.unlink(missing_ok=True)

    def test_exit_2_missing_head_file(self, tmp_path):
        base_p = _write_snapshot(BASE_SNAPSHOT)
        try:
            with pytest.raises(SystemExit) as exc_info:
                main(["--base", str(base_p),
                      "--head", str(tmp_path / "missing.json")])
            assert exc_info.value.code == 2
        finally:
            base_p.unlink(missing_ok=True)

    def test_exit_2_invalid_json_base(self, tmp_path):
        bad = tmp_path / "bad.json"
        bad.write_text("{broken", encoding="utf-8")
        head_p = _write_snapshot(BASE_SNAPSHOT)
        try:
            with pytest.raises(SystemExit) as exc_info:
                main(["--base", str(bad), "--head", str(head_p)])
            assert exc_info.value.code == 2
        finally:
            head_p.unlink(missing_ok=True)

    # --- --quiet suppresses output ---

    def test_quiet_suppresses_stdout(self, capsys):
        base_p = _write_snapshot(BASE_SNAPSHOT)
        head_p = _write_snapshot(HEAD_SNAPSHOT_ADMIN_CHANGED)
        try:
            main(["--base", str(base_p), "--head", str(head_p), "--quiet"])
            captured = capsys.readouterr()
            assert captured.out == ""
        finally:
            base_p.unlink(missing_ok=True)
            head_p.unlink(missing_ok=True)

    def test_quiet_still_returns_correct_exit_code(self):
        base_p = _write_snapshot(BASE_SNAPSHOT)
        head_p = _write_snapshot(HEAD_SNAPSHOT_ADMIN_CHANGED)
        try:
            code = main(["--base", str(base_p), "--head", str(head_p), "--quiet"])
            assert code == 1
        finally:
            base_p.unlink(missing_ok=True)
            head_p.unlink(missing_ok=True)

    # --- --output-format json ---

    def test_json_output_format_is_valid_json(self, capsys):
        base_p = _write_snapshot(BASE_SNAPSHOT)
        head_p = _write_snapshot(HEAD_SNAPSHOT_ADMIN_CHANGED)
        try:
            main(["--base", str(base_p), "--head", str(head_p),
                  "--output-format", "json"])
            captured = capsys.readouterr()
            parsed = json.loads(captured.out)
            assert parsed["flagged"] is True
        finally:
            base_p.unlink(missing_ok=True)
            head_p.unlink(missing_ok=True)

    def test_json_output_no_changes(self, capsys):
        base_p = _write_snapshot(BASE_SNAPSHOT)
        head_p = _write_snapshot(BASE_SNAPSHOT)
        try:
            main(["--base", str(base_p), "--head", str(head_p),
                  "--output-format", "json"])
            captured = capsys.readouterr()
            parsed = json.loads(captured.out)
            assert parsed["flagged"] is False
            assert parsed["security_relevant_changes"] == []
        finally:
            base_p.unlink(missing_ok=True)
            head_p.unlink(missing_ok=True)


# ---------------------------------------------------------------------------
# 10. build_parser — argument validation
# ---------------------------------------------------------------------------

class TestBuildParser:
    """build_parser() produces a parser with the expected arguments."""

    def test_requires_base(self):
        parser = build_parser()
        with pytest.raises(SystemExit):
            parser.parse_args(["--head", "h.json"])

    def test_requires_head(self):
        parser = build_parser()
        with pytest.raises(SystemExit):
            parser.parse_args(["--base", "b.json"])

    def test_accepts_both(self):
        parser = build_parser()
        args = parser.parse_args(["--base", "b.json", "--head", "h.json"])
        assert args.base == "b.json"
        assert args.head == "h.json"

    def test_quiet_default_false(self):
        parser = build_parser()
        args = parser.parse_args(["--base", "b.json", "--head", "h.json"])
        assert args.quiet is False

    def test_quiet_flag_sets_true(self):
        parser = build_parser()
        args = parser.parse_args(["--base", "b.json", "--head", "h.json", "--quiet"])
        assert args.quiet is True

    def test_output_format_default(self):
        parser = build_parser()
        args = parser.parse_args(["--base", "b.json", "--head", "h.json"])
        assert args.output_format == "human"

    def test_output_format_json(self):
        parser = build_parser()
        args = parser.parse_args(
            ["--base", "b.json", "--head", "h.json", "--output-format", "json"]
        )
        assert args.output_format == "json"

    def test_invalid_output_format_rejected(self):
        parser = build_parser()
        with pytest.raises(SystemExit):
            parser.parse_args(
                ["--base", "b.json", "--head", "h.json", "--output-format", "xml"]
            )


# ---------------------------------------------------------------------------
# 11. End-to-end integration: realistic snapshot shapes
# ---------------------------------------------------------------------------

class TestIntegration:
    """Realistic end-to-end scenarios using near-production snapshot shapes."""

    def _run(self, base: dict, head: dict) -> tuple[int, str]:
        """Run main() with tmp files; return (exit_code, stdout)."""
        import io
        from contextlib import redirect_stdout
        base_p = _write_snapshot(base)
        head_p = _write_snapshot(head)
        buf = io.StringIO()
        try:
            with redirect_stdout(buf):
                code = main(["--base", str(base_p), "--head", str(head_p)])
        finally:
            base_p.unlink(missing_ok=True)
            head_p.unlink(missing_ok=True)
        return code, buf.getvalue()

    def test_only_stream_count_changed_is_clean(self):
        head = json.loads(json.dumps(BASE_SNAPSHOT))
        head["next_stream_id"] = 5
        code, _ = self._run(BASE_SNAPSHOT, head)
        assert code == 0

    def test_only_status_change_is_clean(self):
        head = json.loads(json.dumps(BASE_SNAPSHOT))
        head["streams"]["0"]["status"] = "Completed"
        code, _ = self._run(BASE_SNAPSHOT, head)
        assert code == 0

    def test_admin_rotate_flagged(self):
        head = json.loads(json.dumps(BASE_SNAPSHOT))
        head["config"]["admin"] = "GNEWADMIN"
        code, out = self._run(BASE_SNAPSHOT, head)
        assert code == 1
        assert "config.admin" in out

    def test_max_rate_cap_raised_flagged(self):
        head = json.loads(json.dumps(BASE_SNAPSHOT))
        head["config"]["max_rate_per_second"] = 999999
        code, _ = self._run(BASE_SNAPSHOT, head)
        assert code == 1

    def test_creation_pause_enabled_flagged(self):
        head = json.loads(json.dumps(BASE_SNAPSHOT))
        head["creation_paused"] = True
        code, _ = self._run(BASE_SNAPSHOT, head)
        assert code == 1

    def test_nonce_added_flagged(self):
        head = json.loads(json.dumps(BASE_SNAPSHOT))
        head["streams"]["0"]["delegated_nonce"] = 1
        code, _ = self._run(BASE_SNAPSHOT, head)
        assert code == 1

    def test_pending_recipient_update_added_flagged(self):
        head = json.loads(json.dumps(BASE_SNAPSHOT))
        head["streams"]["0"]["pending_recipient_update"] = "GNEWRECIP"
        code, _ = self._run(BASE_SNAPSHOT, head)
        assert code == 1

    def test_token_swap_flagged(self):
        head = json.loads(json.dumps(BASE_SNAPSHOT))
        head["config"]["token"] = "GEVIL_TOKEN"
        code, _ = self._run(BASE_SNAPSHOT, head)
        assert code == 1

    def test_contract_version_bump_flagged(self):
        base = {**BASE_SNAPSHOT, "contract_version": 1}
        head = {**BASE_SNAPSHOT, "contract_version": 2}
        code, _ = self._run(base, head)
        assert code == 1

    def test_multiple_non_security_fields_all_clean(self):
        head = json.loads(json.dumps(BASE_SNAPSHOT))
        head["streams"]["0"]["status"] = "Paused"
        head["streams"]["0"]["start_time"] = 1700000001
        head["streams"]["0"]["withdrawn_amount"] = 100
        head["next_stream_id"] = 10
        code, _ = self._run(BASE_SNAPSHOT, head)
        assert code == 0

    def test_both_security_and_non_security_exits_1(self):
        head = json.loads(json.dumps(BASE_SNAPSHOT))
        head["streams"]["0"]["status"] = "Paused"         # non-security
        head["config"]["admin"] = "GEVIL"                 # security
        code, _ = self._run(BASE_SNAPSHOT, head)
        assert code == 1
