"""Tests for script/validate_migration.py.

These tests validate that docs/MIGRATION.md accurately describes the v1
contract migration path.  They cover:

  - ABI loading and classification helpers
  - Renames-table parsing from the markdown prose
  - Entrypoint-count claim parsing
  - Old-contract-count claim parsing
  - Each validation check function, both passing and failing paths
  - End-to-end validation() against the real repo fixtures
"""

import importlib.util
import json
import sys
from pathlib import Path
from unittest.mock import patch

import pytest

REPO_ROOT = Path(__file__).resolve().parent.parent

SCRIPT = REPO_ROOT / "script" / "validate_migration.py"
SPEC = importlib.util.spec_from_file_location("validate_migration", SCRIPT)
vm = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(vm)


# ---------------------------------------------------------------------------
# Fixtures
# ---------------------------------------------------------------------------

@pytest.fixture
def real_abi():
    return vm.load_abi()


@pytest.fixture
def real_doc():
    return vm.parse_migration_doc()


@pytest.fixture
def sample_doc():
    """A minimal MIGRATION.md that exercises all parser functions."""
    return """\
# Migration: `main` → `v1-rewrite`

## 3. Behaviour deliberately removed

The old contract set exposed **145 entrypoints** (100 stream, 16 factory, 29
governance). v1 exposes **16**.

`init`, `set_admin`, `upgrade`, `version`, `pause_protocol`,
`resume_protocol`, `global_resume`, `set_global_emergency_paused`,
`get_global_emergency_paused`, `set_contract_paused`, `is_paused`,
`cancel_stream_as_admin`, `pause_stream_as_admin`, `resume_stream_as_admin`,
`bulk_resume_streams_as_admin`, `set_stream_decommissioned`, `sweep_excess`,
`get_protocol_fees_accrued`, `get_keeper_fee_split`, `set_max_rate_per_second`.

## 4. Renames — the part most likely to bite

| old | v1 | change |
|---|---|---|
| `cancel_stream(sender, id)` | `cancel(id)` | renamed; `sender` dropped |
| `withdraw(recipient, id, amount)` | `withdraw(id, amount: Option<i128>)` | 3 args → 2 |
| `create_stream(sender, recipient, amount, start, end, cliff)` | `create_stream(sender, recipient, token, deposit, start, end, cliff, cancellable, pausable, transferable)` | 6 args → 10 |
"""


@pytest.fixture
def sample_abi():
    """A minimal ABI with 3 core + 2 delegation entrypoints."""
    return {
        "abi_version": 1,
        "functions": [
            {
                "name": "create_stream",
                "auth": "sender",
                "inputs": [
                    {"name": "sender", "type": "Address"},
                    {"name": "recipient", "type": "Address"},
                    {"name": "token", "type": "Address"},
                    {"name": "deposit", "type": "i128"},
                    {"name": "start_time", "type": "u64"},
                    {"name": "end_time", "type": "u64"},
                    {"name": "cliff_time", "type": "u64"},
                    {"name": "cancellable", "type": "bool"},
                    {"name": "pausable", "type": "bool"},
                    {"name": "transferable", "type": "bool"},
                ],
                "outputs": "Result<u64, Error>",
            },
            {
                "name": "cancel",
                "auth": "sender",
                "inputs": [{"name": "stream_id", "type": "u64"}],
                "outputs": "Result<(), Error>",
            },
            {
                "name": "withdraw",
                "auth": "recipient",
                "inputs": [
                    {"name": "stream_id", "type": "u64"},
                    {"name": "amount", "type": "Option<i128>"},
                ],
                "outputs": "Result<i128, Error>",
            },
            {
                "name": "top_up",
                "auth": "sender",
                "inputs": [
                    {"name": "stream_id", "type": "u64"},
                    {"name": "amount", "type": "i128"},
                ],
                "outputs": "Result<(), Error>",
            },
            {
                "name": "delegate_withdraw",
                "auth": "delegate",
                "inputs": [
                    {"name": "stream_id", "type": "u64"},
                    {"name": "delegate", "type": "Address"},
                    {"name": "amount", "type": "Option<i128>"},
                ],
                "outputs": "Result<i128, Error>",
            },
            {
                "name": "grant_delegate",
                "auth": "grantor",
                "inputs": [
                    {"name": "stream_id", "type": "u64"},
                    {"name": "grantor", "type": "Address"},
                    {"name": "delegate", "type": "Address"},
                    {"name": "ops", "type": "u32"},
                    {"name": "expires_at", "type": "Option<u64>"},
                ],
                "outputs": "Result<(), Error>",
            },
        ],
        "types": [],
        "errors": [],
        "events": [],
    }


# ---------------------------------------------------------------------------
# ABI helpers
# ---------------------------------------------------------------------------

class TestAbiLoading:
    def test_load_abi_returns_dict(self, real_abi):
        assert isinstance(real_abi, dict)
        assert "functions" in real_abi

    def test_load_abi_file_not_found(self, tmp_path):
        with pytest.raises(FileNotFoundError):
            vm.load_abi(tmp_path / "nonexistent.json")

    def test_abi_entrypoints_returns_sorted_names(self, real_abi):
        names = vm.abi_entrypoints(real_abi)
        assert names == sorted(names)
        assert "create_stream" in names
        assert "cancel" in names

    def test_abi_function_finds_entry(self, real_abi):
        fn = vm.abi_function(real_abi, "create_stream")
        assert fn is not None
        assert fn["auth"] == "sender"

    def test_abi_function_returns_none_for_missing(self, real_abi):
        assert vm.abi_function(real_abi, "cancel_stream") is None

    def test_abi_input_types(self, real_abi):
        types = vm.abi_input_types(real_abi, "withdraw")
        assert types == ["u64", "Option<i128>"]

    def test_abi_input_types_for_missing(self, real_abi):
        assert vm.abi_input_types(real_abi, "nonexistent") == []

    def test_classify_entrypoints(self, real_abi):
        core, delegation = vm.classify_entrypoints(real_abi)
        assert "cancel" in core
        assert "create_stream" in core
        assert "delegate_withdraw" in delegation
        assert "grant_delegate" in delegation
        assert "revoke_delegate" in delegation
        assert core.isdisjoint(delegation)


# ---------------------------------------------------------------------------
# MIGRATION.md parsing
# ---------------------------------------------------------------------------

class TestParseMigrationDoc:
    def test_returns_string(self, real_doc):
        assert isinstance(real_doc, str)
        assert "Migration" in real_doc

    def test_missing_file_raises(self, tmp_path):
        with pytest.raises(FileNotFoundError):
            vm.parse_migration_doc(tmp_path / "missing.md")


class TestParseRenamesTable:
    def test_parses_all_renames(self, real_doc):
        renames = vm.parse_renames_table(real_doc)
        assert len(renames) == 11
        # Spot-check a few.
        assert ("cancel_stream", "cancel") in renames
        assert ("pause_stream", "pause") in renames
        assert ("update_recipient", "transfer_recipient") in renames
        assert ("get_stream_count", "stream_count") in renames
        assert ("calculate_accrued", "vested_of") in renames

    def test_parses_same_name_signature_changes(self, real_doc):
        renames = vm.parse_renames_table(real_doc)
        assert ("create_stream", "create_stream") in renames
        assert ("withdraw", "withdraw") in renames

    def test_extract_fn_name_strips_params(self):
        result = vm._extract_fn_name("`cancel_stream(sender, id)`")
        assert result == "cancel_stream"

    def test_extract_fn_name_handles_slash(self):
        result = vm._extract_fn_name("`update_recipient` / `accept_recipient_update`")
        assert result == "update_recipient"

    def test_extract_fn_name_handles_bare_name(self):
        result = vm._extract_fn_name("`stream_count()`")
        assert result == "stream_count"

    def test_raises_on_missing_table(self):
        with pytest.raises(ValueError, match="renames table"):
            vm.parse_renames_table("# Some doc\n\nNo renames here.")

    def test_parses_sample_doc(self, sample_doc):
        renames = vm.parse_renames_table(sample_doc)
        assert ("cancel_stream", "cancel") in renames
        assert ("withdraw", "withdraw") in renames
        assert ("create_stream", "create_stream") in renames


class TestParseEntrypointCountClaim:
    def test_extracts_v1_count(self, real_doc):
        count = vm.parse_entrypoint_count_claim(real_doc)
        assert count == 17

    def test_returns_none_when_absent(self):
        assert vm.parse_entrypoint_count_claim("# No counts here") is None

    def test_parses_sample_doc(self, sample_doc):
        assert vm.parse_entrypoint_count_claim(sample_doc) == 16


class TestParseOldCountClaim:
    def test_extracts_old_count(self, real_doc):
        count = vm.parse_old_count_claim(real_doc)
        assert count == 145

    def test_returns_none_when_absent(self):
        assert vm.parse_old_count_claim("# No counts here") is None

    def test_parses_sample_doc(self, sample_doc):
        assert vm.parse_old_count_claim(sample_doc) == 145


# ---------------------------------------------------------------------------
# Validation checks
# ---------------------------------------------------------------------------

class TestCheckRenames:
    def test_passes_with_real_files(self, real_abi, real_doc):
        errors = vm.check_renames(real_abi, real_doc)
        assert errors == []

    def test_fails_when_old_name_still_exists(self, sample_abi, sample_doc):
        # Add the old function name to the ABI.
        sample_abi["functions"].append({
            "name": "cancel_stream",
            "auth": "sender",
            "inputs": [{"name": "id", "type": "u64"}],
            "outputs": "Result<(), Error>",
        })
        errors = vm.check_renames(sample_abi, sample_doc)
        assert any("cancel_stream" in e for e in errors)

    def test_fails_when_new_name_missing(self, sample_abi, sample_doc):
        # Remove the renamed function.
        sample_abi["functions"] = [
            f for f in sample_abi["functions"] if f["name"] != "cancel"
        ]
        errors = vm.check_renames(sample_abi, sample_doc)
        assert any("`cancel`" in e for e in errors)

    def test_fails_on_unparseable_table(self, sample_abi):
        errors = vm.check_renames(sample_abi, "# No renames table here")
        assert any("renames table" in e for e in errors)

    def test_empty_renames_reported(self, sample_abi):
        doc = "## 4. Renames\n\n| old | v1 | change |\n|---|---|---|\n"
        errors = vm.check_renames(sample_abi, doc)
        assert any("empty" in e.lower() for e in errors)


class TestCheckRemoved:
    def test_passes_with_real_abi(self, real_abi):
        errors = vm.check_removed(real_abi)
        assert errors == []

    def test_fails_when_removed_fn_present(self, sample_abi):
        # Add a removed entrypoint to the ABI.
        sample_abi["functions"].append({
            "name": "init",
            "auth": "sender",
            "inputs": [],
            "outputs": "Result<(), Error>",
        })
        errors = vm.check_removed(sample_abi)
        assert any("`init`" in e for e in errors)

    def test_fails_for_multiple_removed_fns(self, sample_abi):
        for name in ["init", "upgrade", "set_admin"]:
            sample_abi["functions"].append({
                "name": name, "auth": "none", "inputs": [],
                "outputs": "Result<(), Error>",
            })
        errors = vm.check_removed(sample_abi)
        assert len(errors) == 3
        for name in ["init", "upgrade", "set_admin"]:
            assert any(f"`{name}`" in e for e in errors)

    def test_removed_set_is_non_empty(self):
        assert len(vm.REMOVED_ENTRYPOINTS) >= 50


class TestCheckSignatures:
    def test_passes_with_real_abi(self, real_abi):
        errors = vm.check_signatures(real_abi)
        assert errors == []

    def test_fails_on_wrong_type(self, sample_abi):
        # Change withdraw's amount type to u64 (the old type).
        for f in sample_abi["functions"]:
            if f["name"] == "withdraw":
                f["inputs"][1]["type"] = "u64"
        errors = vm.check_signatures(sample_abi)
        assert any("Option<i128>" in e for e in errors)

    def test_fails_on_missing_param(self, sample_abi):
        # Remove the 'token' param from create_stream.
        for f in sample_abi["functions"]:
            if f["name"] == "create_stream":
                f["inputs"] = [
                    p for p in f["inputs"] if p["name"] != "token"
                ]
        errors = vm.check_signatures(sample_abi)
        assert any("token" in e for e in errors)

    def test_fails_on_missing_function(self, sample_abi):
        sample_abi["functions"] = [
            f for f in sample_abi["functions"] if f["name"] != "create_stream"
        ]
        errors = vm.check_signatures(sample_abi)
        assert any("create_stream" in e for e in errors)


class TestCheckEntrypointCount:
    def test_passes_with_real_files(self, real_abi, real_doc):
        errors = vm.check_entrypoint_count(real_abi, real_doc)
        assert errors == []

    def test_fails_when_count_wrong(self, sample_abi, sample_doc):
        # The sample doc claims 16, but sample_abi has 4 core entrypoints.
        errors = vm.check_entrypoint_count(sample_abi, sample_doc)
        assert len(errors) == 1
        assert "16" in errors[0]
        assert "4" in errors[0]

    def test_fails_when_no_count_claim(self, sample_abi):
        doc = "# Doc without a count claim\n"
        errors = vm.check_entrypoint_count(sample_abi, doc)
        assert any("count claim" in e.lower() for e in errors)


class TestCheckOldCountClaim:
    def test_passes_with_real_doc(self, real_doc):
        errors = vm.check_old_count_claim(real_doc)
        assert errors == []

    def test_fails_on_wrong_breakdown(self):
        doc = "exposed **150 entrypoints** (100 stream, 25 factory, 29 governance)."
        errors = vm.check_old_count_claim(doc)
        assert len(errors) == 1
        assert "150" in errors[0]
        assert "145" in errors[0]

    def test_fails_on_missing_claim(self):
        errors = vm.check_old_count_claim("# No count")
        assert len(errors) == 1
        assert "old entrypoint count" in errors[0]


# ---------------------------------------------------------------------------
# End-to-end validate()
# ---------------------------------------------------------------------------

class TestValidate:
    def test_passes_with_real_repo(self):
        errors = vm.validate()
        assert errors == [], f"Validation errors: {errors}"

    def test_fails_on_missing_abi(self, tmp_path, real_doc):
        errors = vm.validate(
            abi_path=tmp_path / "nonexistent.json",
            migration_path=None,
        )
        assert len(errors) == 1
        assert "not found" in errors[0]

    def test_fails_on_missing_doc(self, real_abi, tmp_path):
        # Write ABI to temp, use missing migration path.
        abi_file = tmp_path / "abi.json"
        abi_file.write_text(json.dumps(real_abi), encoding="utf-8")
        errors = vm.validate(
            abi_path=abi_file,
            migration_path=tmp_path / "nonexistent.md",
        )
        assert len(errors) == 1
        assert "not found" in errors[0]

    def test_returns_all_errors(self, tmp_path):
        """When multiple checks fail, all errors are returned."""
        abi_file = tmp_path / "abi.json"
        doc_file = tmp_path / "MIGRATION.md"

        # ABI with a removed entrypoint and a missing renamed function.
        abi = {
            "abi_version": 1,
            "functions": [
                {"name": "init", "auth": "none", "inputs": [],
                 "outputs": "Result<(), Error>"},
            ],
            "types": [], "errors": [], "events": [],
        }
        abi_file.write_text(json.dumps(abi), encoding="utf-8")

        doc = (
            "# Migration\n\n"
            "## 3. Behaviour deliberately removed\n\n"
            "exposed **145 entrypoints** (100 stream, 16 factory, 29 governance). "
            "v1 exposes **1**.\n\n"
            "## 4. Renames\n\n"
            "| old | v1 | change |\n|---|---|---|\n"
            "| `cancel_stream(sender, id)` | `cancel(id)` | renamed |\n"
        )
        doc_file.write_text(doc, encoding="utf-8")

        errors = vm.validate(abi_path=abi_file, migration_path=doc_file)
        # Should have: removal violation (init), rename violation (cancel missing),
        # signature check failures, count mismatch, and old count mismatch.
        assert len(errors) > 1


# ---------------------------------------------------------------------------
# Main entry point
# ---------------------------------------------------------------------------

class TestMain:
    def test_exits_zero_when_ok(self, capsys):
        with patch.object(vm, "validate", return_value=[]):
            code = vm.main()
        assert code == 0
        out = capsys.readouterr().out
        assert "consistent" in out

    def test_exits_one_when_errors(self, capsys):
        with patch.object(vm, "validate", return_value=["some error"]):
            code = vm.main()
        assert code == 1
        out = capsys.readouterr().out
        assert "FAIL" in out
        assert "some error" in out
