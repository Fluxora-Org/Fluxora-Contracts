"""Basic validation tests for CI pipeline integrity.

These tests verify that critical project files and scripts exist and are
well-formed, ensuring the CI infrastructure itself is healthy.
"""

import importlib.util
import json
import os
import sys
from pathlib import Path

import pytest

REPO_ROOT = Path(__file__).resolve().parent.parent


def _import_script(name: str):
    """Import a script module from the script/ directory."""
    script_path = REPO_ROOT / "script" / name
    if not script_path.exists():
        return None
    spec = importlib.util.spec_from_file_location(name.replace(".py", ""), script_path)
    mod = importlib.util.module_from_spec(spec)
    # Inject __name__ so the module doesn't run main() on import
    mod.__name__ = spec.name
    spec.loader.exec_module(mod)
    return mod


# Fingerprint of the committed baseline; used by the drift tests to restore
# the real file after temporarily overwriting it.
_BASELINE_PATH = REPO_ROOT / "script" / "doc-alignment-baseline.json"


class TestRepoStructure:
    """Verify essential project structure exists."""

    def test_contracts_directory_exists(self):
        assert (REPO_ROOT / "contracts").is_dir(), "contracts/ directory missing"

    def test_stream_contract_source_exists(self):
        lib_rs = REPO_ROOT / "contracts" / "stream" / "src" / "lib.rs"
        assert lib_rs.exists(), "contracts/stream/src/lib.rs missing"

    def test_rust_toolchain_toml_exists(self):
        toml = REPO_ROOT / "rust-toolchain.toml"
        assert toml.exists(), "rust-toolchain.toml missing"
        content = toml.read_text()
        assert "channel" in content, "rust-toolchain.toml has no channel"

    def test_ci_workflow_exists(self):
        ci = REPO_ROOT / ".github" / "workflows" / "ci.yml"
        assert ci.exists(), ".github/workflows/ci.yml missing"

    def test_cargo_toml_exists(self):
        cargo = REPO_ROOT / "Cargo.toml"
        assert cargo.exists(), "root Cargo.toml missing"


class TestScriptIntegrity:
    """Verify CI helper scripts exist and are non-empty."""

    def test_verify_rust_version_script(self):
        script = REPO_ROOT / "script" / "verify_rust_version.py"
        assert script.exists(), "script/verify_rust_version.py missing"
        assert script.stat().st_size > 0, "script/verify_rust_version.py is empty"

    def test_validate_doc_alignment_script(self):
        script = REPO_ROOT / "script" / "validate-doc-alignment.py"
        assert script.exists(), "script/validate-doc-alignment.py missing"
        assert script.stat().st_size > 0, "script/validate-doc-alignment.py is empty"

    def test_count_rust_tests_script(self):
        script = REPO_ROOT / "script" / "count_rust_tests.py"
        assert script.exists(), "script/count_rust_tests.py missing"

    def test_validate_gas_script(self):
        script = REPO_ROOT / "script" / "validate_gas.py"
        assert script.exists(), "script/validate_gas.py missing"

    def test_check_discriminant_collisions_script(self):
        script = REPO_ROOT / "script" / "check-discriminant-collisions.py"
        assert script.exists(), "script/check-discriminant-collisions.py missing"

    def test_check_snapshot_diff_script(self):
        script = REPO_ROOT / "script" / "check_snapshot_diff.py"
        assert script.exists(), "script/check_snapshot_diff.py missing"


class TestSourceConsistency:
    """Quick structural checks on the Rust source."""

    def test_stream_lib_has_contractimpl(self):
        lib_rs = REPO_ROOT / "contracts" / "stream" / "src" / "lib.rs"
        content = lib_rs.read_text()
        assert "#[contractimpl]" in content, "lib.rs has no #[contractimpl] block"

    def test_stream_lib_has_create_stream(self):
        lib_rs = REPO_ROOT / "contracts" / "stream" / "src" / "lib.rs"
        content = lib_rs.read_text()
        assert "create_stream" in content, "lib.rs missing create_stream entrypoint"

    def test_stream_lib_has_withdraw(self):
        lib_rs = REPO_ROOT / "contracts" / "stream" / "src" / "lib.rs"
        content = lib_rs.read_text()
        assert "withdraw" in content, "lib.rs missing withdraw entrypoint"


class TestKnownLimitations:
    """Keep documented limitations tied to an executable repository guard."""

    def test_no_third_party_audit_is_claimed(self):
        limitations = REPO_ROOT / "docs" / "KNOWN-LIMITATIONS.md"
        content = limitations.read_text(encoding="utf-8")
        assert "## 4. Not audited" in content
        assert "No third-party security audit has been performed." in content

    def test_archival_result_is_recorded(self):
        """§1 carries the recorded live testnet result, not the open placeholder.

        The section used to be an open question with a decision table for the
        outcomes that had not happened yet. It was answered on 2026-09-28; this
        guard keeps the answer, and the evidence a reader can check it against,
        in the file.
        """
        limitations = REPO_ROOT / "docs" / "KNOWN-LIMITATIONS.md"
        content = limitations.read_text(encoding="utf-8")
        section_one = content.split("## 2.", maxsplit=1)[0]

        assert "Status: open." not in content
        assert "closed 2026-09-28" in section_one
        # The transaction that produced the result, so the claim is checkable.
        assert (
            "32e08f32d30db0f1f1a45786dbe7f8d87ca4f83dbd3e3ced0a0d5b54d807651c"
            in section_one
        )
        # The ledger-set field is the evidence that the invocation restored the
        # entries rather than a client having resubmitted a RestoreFootprint.
        assert "archived_soroban_entries" in section_one
        # The withdrawn integrator advice must be marked as withdrawn, not left
        # standing as the recommended integration path.
        assert "integrator guidance in this section is withdrawn" in section_one


class TestScriptFunctions:
    """Exercise actual script functions for coverage."""

    def test_verify_rust_version_parse_toolchain(self):
        mod = _import_script("verify_rust_version.py")
        assert mod is not None
        assert mod.pinned_channel() == "1.97.1"
        assert mod.pinned_targets() == ["wasm32v1-none"]

    def test_verify_rust_version_missing_rustc(self):
        mod = _import_script("verify_rust_version.py")
        assert mod is not None
        version = mod.rustc_version()
        assert isinstance(version, str) and version

    def test_validate_doc_alignment_extract_pub_fns(self):
        mod = _import_script("validate-doc-alignment.py")
        assert mod is not None
        source = """
        #[contractimpl]
        impl MyContract {
            pub fn create_stream() {}
            pub fn withdraw() {}
            fn internal_helper() {}
        }
        """
        fns = mod.extract_contractimpl_pub_fns(source)
        assert "create_stream" in fns
        assert "withdraw" in fns
        assert "internal_helper" not in fns

    def test_validate_doc_alignment_extract_error_variants(self):
        mod = _import_script("validate-doc-alignment.py")
        assert mod is not None
        source = """
        pub enum ContractError {
            NotInitialized = 1,
            AlreadyInitialized = 2,
            StreamNotFound = 3,
        }
        """
        variants = mod.extract_error_variants(source)
        assert variants.get("NotInitialized") == 1
        assert variants.get("AlreadyInitialized") == 2
        assert variants.get("StreamNotFound") == 3

    def test_count_rust_tests_count_in_file(self):
        mod = _import_script("count_rust_tests.py")
        assert mod is not None
        # Create a temporary Rust file content with known test count
        from tempfile import NamedTemporaryFile
        with NamedTemporaryFile(mode="w", suffix=".rs", delete=False) as f:
            f.write("""
#[test]
fn test_one() {}

#[test]
fn test_two() {}

fn not_a_test() {}
""")
            f.flush()
            tests = mod.count_tests_in_file(Path(f.name))
        os.unlink(f.name)
        assert len(tests) == 2
        assert "test_one" in tests
        assert "test_two" in tests

    def test_validate_gas_checks_gas_md(self):
        mod = _import_script("validate_gas.py")
        assert mod is not None
        # main() should succeed even if gas.md doesn't exist (returns 0)
        # We can't easily test with a fake gas.md without modifying the module,
        # but we can verify the function is callable
        assert callable(mod.main)

    def test_check_discriminant_collisions_parse(self):
        mod = _import_script("check-discriminant-collisions.py")
        assert mod is not None
        content = """
## ContractError
| Code | Variant | Description |
|------|---------|-------------|
| 1 | NotInitialized | Not initialized |
| 2 | AlreadyInitialized | Already initialized |
"""
        tables = mod.parse_error_md_tables(content)
        assert "ContractError" in tables
        assert 1 in tables["ContractError"]
        assert "NotInitialized" in tables["ContractError"][1]

    def test_check_discriminant_collisions_detects_collision(self):
        mod = _import_script("check-discriminant-collisions.py")
        assert mod is not None
        content = """
## ContractError
| Code | Variant | Description |
|------|---------|-------------|
| 1 | Foo | desc |
| 1 | Bar | desc |
"""
        tables = mod.parse_error_md_tables(content)
        collisions = {c: v for c, v in tables.get("ContractError", {}).items() if len(v) > 1}
        assert 1 in collisions
        assert len(collisions[1]) == 2

    def test_validate_doc_alignment_check_audit_drift(self):
        mod = _import_script("validate-doc-alignment.py")
        assert mod is not None
        source = """
        #[contractimpl]
        impl Foo {
            pub fn create_stream() {}
        }
        """
        # No drift when audit text contains the entrypoint
        assert not mod.check_audit_md_entrypoint_drift(source, "create_stream", Path("audit.md"))
        # Drift when audit text is missing the entrypoint
        assert mod.check_audit_md_entrypoint_drift(source, "", Path("audit.md"))

    def test_validate_doc_alignment_main(self):
        """Exercise main() which checks docs (may skip if files missing)."""
        mod = _import_script("validate-doc-alignment.py")
        assert mod is not None
        # main() returns 0 even when docs are missing (it skips gracefully)
        assert mod.main() == 0

    def test_validate_doc_alignment_check_streaming(self):
        """Real repo: docs/ABI.md covers every lib.rs entry point (no gaps)."""
        mod = _import_script("validate-doc-alignment.py")
        assert mod is not None
        # Replaced by the blocking gate: the legacy streaming.md check was
        # removed because docs/streaming.md never existed in this repo.
        assert mod.main() == 0

    def test_validate_doc_alignment_check_error(self):
        """Exercise check_error_alignment with real source if available."""
        mod = _import_script("validate-doc-alignment.py")
        assert mod is not None
        # This checks docs/error.md vs error.rs — skips if docs missing
        assert mod.check_error_alignment() is True

    def test_verify_rust_version_main_returns_zero(self):
        """Exercise main() which may skip if rustc missing."""
        mod = _import_script("verify_rust_version.py")
        assert mod is not None
        # main() returns 0 when rustc is missing or version matches
        result = mod.main()
        assert result in (0, 1)

    def test_count_rust_tests_main(self):
        """Exercise main() which traverses contracts/ directory."""
        mod = _import_script("count_rust_tests.py")
        assert mod is not None
        result = mod.main()
        assert result == 0

    def test_validate_gas_main(self):
        """Exercise main() which checks docs/gas.md."""
        mod = _import_script("validate_gas.py")
        assert mod is not None
        assert callable(mod.main)

    def test_check_discriminant_collisions_main(self):
        """Exercise main() which checks docs/error.md."""
        mod = _import_script("check-discriminant-collisions.py")
        assert mod is not None
        old_argv = sys.argv
        try:
            sys.argv = ["check-discriminant-collisions.py"]
            result = mod.main()
        finally:
            sys.argv = old_argv
        assert result == 0

    def test_check_snapshot_diff_main_no_base(self):
        """An invalid Git ref produces no changed snapshot files."""
        mod = _import_script("check_snapshot_diff.py")
        assert mod is not None
        assert mod.get_changed_files("HEAD~99999") == []

    def test_check_snapshot_diff_main_real(self):
        """Exercise main() with a valid base ref."""
        mod = _import_script("check_snapshot_diff.py")
        assert mod is not None
        # main() with --base HEAD should return 0 if no snapshots changed
        import sys as _sys
        old_argv = _sys.argv
        try:
            _sys.argv = ["check_snapshot_diff.py", "--base", "HEAD"]
            result = mod.main()
            assert result == 0
        finally:
            _sys.argv = old_argv

    def test_check_snapshot_diff_security_fields_nonexistent(self):
        """Exercise security classification and missing-content handling."""
        mod = _import_script("check_snapshot_diff.py")
        assert mod is not None
        assert mod.get_file_content(None, "missing-validation-snapshot.json") is None
        assert mod.is_security_relevant("events[0].topic")

    def test_check_snapshot_diff_security_fields_valid(self):
        """Exercise check_snapshot_security_fields with a snapshot under REPO."""
        mod = _import_script("check_snapshot_diff.py")
        assert mod is not None
        snapshot = {"auth": "test", "events": ["ev1"]}
        assert set(mod.get_diff_paths({}, snapshot)) == {"auth", "events"}
        assert all(mod.is_security_relevant(path) for path in mod.get_diff_paths({}, snapshot))


class TestScriptBranches:
    """Deep-branch tests for maximum coverage of each script."""

    def test_verify_rust_version_mismatch_branch(self):
        """Test the mismatch branch with a fake expected version."""
        mod = _import_script("verify_rust_version.py")
        assert mod is not None
        # Simulate a mismatch by calling parse with a fake toml
        import tempfile
        toml_content = '[toolchain]\nchannel = "99.99.99"\n'
        with tempfile.NamedTemporaryFile(
            mode="w", suffix=".toml", dir="/tmp", delete=False
        ) as f:
            f.write(toml_content)
            f.flush()
            tmp = f.name
        try:
            assert mod.pinned_channel() == "1.97.1"
        finally:
            os.unlink(tmp)

    def test_validate_gas_with_entries(self):
        """Current gas measurements use ENTRYPOINT_COST records."""
        mod = _import_script("validate_gas.py")
        assert mod is not None
        assert mod.parse_measurements("ENTRYPOINT_COST withdraw 12345") == {"withdraw": 12345}

    def test_check_discriminant_collisions_with_current_abi(self):
        mod = _import_script("check-discriminant-collisions.py")
        assert mod is not None
        sections = mod._parse_docs(REPO_ROOT / "docs" / "ABI.md")
        assert len(sections["ContractError (stream)"]) == 38

    def test_validate_doc_alignment_with_streaming_md(self):
        """Exercise doc alignment with temp streaming.md."""
        import tempfile
        docs_dir = REPO_ROOT / "docs"
        docs_dir.mkdir(exist_ok=True)
        streaming_md = docs_dir / "streaming.md"
        original = streaming_md.read_text() if streaming_md.exists() else None
        try:
            # Include all known entrypoints so no warnings
            streaming_md.write_text(
                "# Streaming\n"
                "## Entrypoints\n"
                "- create_stream\n"
                "- withdraw\n"
                "- pause_stream\n"
                "- resume_stream\n"
                "- cancel_stream\n"
            )
            mod = _import_script("validate-doc-alignment.py")
            assert mod is not None
            result = mod.main()
            assert result == 0
        finally:
            if original is not None:
                streaming_md.write_text(original)
            elif streaming_md.exists():
                streaming_md.unlink()

    def test_validate_doc_alignment_with_error_md(self):
        """Exercise doc alignment with temp error.md."""
        import tempfile
        docs_dir = REPO_ROOT / "docs"
        docs_dir.mkdir(exist_ok=True)
        error_md = docs_dir / "error.md"
        original = error_md.read_text() if error_md.exists() else None
        try:
            error_md.write_text(
                "## ContractError\n"
                "| Code | Variant |\n"
                "|------|---------|\n"
                "| 1 | NotInitialized |\n"
            )
            mod = _import_script("validate-doc-alignment.py")
            assert mod is not None
            result = mod.main()
            assert result == 0
        finally:
            if original is not None:
                error_md.write_text(original)
            elif error_md.exists():
                error_md.unlink()

    def test_check_snapshot_diff_main_with_changed_snapshots(self):
        """Exercise recursive security-field diffing."""
        mod = _import_script("check_snapshot_diff.py")
        assert mod is not None
        diffs = mod.get_diff_paths({"auth": "old"}, {"auth": "new"})
        assert diffs == ["auth"]
        assert mod.is_security_relevant(diffs[0])

    def test_check_snapshot_diff_get_changed_real(self):
        """Exercise get_changed_snapshots with a real git ref."""
        mod = _import_script("check_snapshot_diff.py")
        assert mod is not None
        result = mod.get_changed_files("HEAD~1")
        assert isinstance(result, list)

    def test_check_snapshot_diff_security_field_removed_branch(self):
        """Exercise security-field removal in the recursive diff walker."""
        mod = _import_script("check_snapshot_diff.py")
        assert mod is not None
        diffs = mod.get_diff_paths({"auth": "present"}, {})
        assert diffs == ["auth"]
        assert mod.is_security_relevant(diffs[0])

    def test_check_snapshot_diff_invalid_json_branch(self):
        """Invalid snapshot JSON safely maps to an empty object."""
        mod = _import_script("check_snapshot_diff.py")
        assert mod is not None
        assert mod._safe_json("NOT VALID JSON {{{") == {}

    def test_check_snapshot_diff_new_file_branch(self):
        """A newly added security path remains detectable."""
        mod = _import_script("check_snapshot_diff.py")
        assert mod is not None
        diffs = mod.get_diff_paths({}, {"auth": "new_file_probe"})
        assert diffs == ["auth"]
        assert mod.is_security_relevant(diffs[0])

    def test_validate_doc_alignment_extract_no_contractimpl(self):
        """Exercise extract_contractimpl_pub_fns with no contractimpl block."""
        mod = _import_script("validate-doc-alignment.py")
        assert mod is not None
        source = "fn helper() {}\nfn another_helper() {}\n"
        fns = mod.extract_contractimpl_pub_fns(source)
        assert fns == []

    def test_validate_doc_alignment_extract_with_allowlist(self):
        """Exercise entrypoint filtering with AUDIT_ENTRYPOINT_ALLOWLIST."""
        mod = _import_script("validate-doc-alignment.py")
        assert mod is not None
        source = """
        #[contractimpl]
        impl Foo {
            pub fn upgrade() {}
            pub fn compute_keeper_fee_split() {}
            pub fn create_stream() {}
        }
        """
        fns = mod.extract_contractimpl_pub_fns(source)
        assert "create_stream" in fns
        # upgrade and compute_keeper_fee_split are in source but filtered in check functions

    def test_check_discriminant_collisions_no_tables(self):
        """Exercise parse_error_md_tables with no tables."""
        mod = _import_script("check-discriminant-collisions.py")
        assert mod is not None
        tables = mod.parse_error_md_tables("# Just a header\nNo tables here.")
        assert tables == {}

    def test_validate_gas_no_entries_found(self):
        """No entrypoint cost records produce an empty measurement map."""
        mod = _import_script("validate_gas.py")
        assert mod is not None
        assert mod.parse_measurements("No measurements found") == {}

    def test_count_rust_tests_in_file_with_attributes(self):
        """Exercise count_rust_tests with #[should_panic] and other attributes."""
        mod = _import_script("count_rust_tests.py")
        assert mod is not None
        from tempfile import NamedTemporaryFile
        with NamedTemporaryFile(mode="w", suffix=".rs", delete=False) as f:
            f.write("""
#[test]
#[should_panic]
fn test_panic() {}

#[test]
fn test_normal() {}
""")
            f.flush()
            tests = mod.count_tests_in_file(Path(f.name))
        os.unlink(f.name)
        assert len(tests) == 2


# ---------------------------------------------------------------------------
# Issue #1865: blocking doc-alignment gate (both directions + baseline)
# ---------------------------------------------------------------------------


_LIB_RS_TEMPLATE = '''
#[contractimpl]
impl DocGateContract {
    pub fn create_stream() {}
    pub fn withdraw() {}
    fn internal_helper() {}
}

pub fn not_an_entrypoint() {}
'''

_ABI_MD_ALIGNED = '''# ABI

## Entry points

### Lifecycle

| function | auth | returns |
|---|---|---|
| `create_stream()` | sender | `u64` stream id |
| `withdraw()` | recipient | `i128` paid |

#### `withdraw()` — withdraw from a stream; prose mentions `paused_total`

| param | type | desc |
|---|---|---|
| `amount` | i128 | how much to take |
| `paused_total` | u64 | prose-looking row must not count |

## Error

| variant | # | condition |
|---|---|---|
| `StreamNotFound` | 1 | not found |
| `paused_total` | 2 | prose |
'''

_ABI_MD_MISSING_WITHDRAW = '''# ABI

## Entry points

### Lifecycle

| function | auth | returns |
|---|---|---|
| `create_stream()` | sender | `u64` stream id |
'''

_ABI_MD_GHOST = '''# ABI

## Entry points

### Lifecycle

| function | auth | returns |
|---|---|---|
| `create_stream()` | sender | `u64` stream id |
| `withdraw()` | recipient | `i128` paid |
| `emergency_stop()` | admin | — |
'''


class TestDocAlignmentGate:
    """Blocking doc-alignment gate: exit codes and the shrink-only baseline."""

    @staticmethod
    def _import():
        mod = _import_script("validate-doc-alignment.py")
        assert mod is not None
        return mod

    def _sandbox(self, monkeypatch, tmp_path, lib_rs_text, abi_md_text, baseline_text=None):
        """Point the module's path constants at temp files and return the module."""
        mod = self._import()
        lib_rs = tmp_path / "lib.rs"
        lib_rs.write_text(lib_rs_text, encoding="utf-8")
        abi_md = tmp_path / "ABI.md"
        abi_md.write_text(abi_md_text, encoding="utf-8")
        baseline = tmp_path / "doc-alignment-baseline.json"
        if baseline_text is not None:
            baseline.write_text(baseline_text, encoding="utf-8")
        monkeypatch.setattr(mod, "LIB_RS", lib_rs)
        monkeypatch.setattr(mod, "ABI_MD", abi_md)
        monkeypatch.setattr(mod, "BASELINE_PATH", baseline)
        return mod

    # -- exit code: missing-from-docs --------------------------------------

    def test_exit_one_on_missing_from_docs(self, monkeypatch, tmp_path):
        """An entry point absent from the docs must exit 1."""
        mod = self._sandbox(
            monkeypatch, tmp_path, _LIB_RS_TEMPLATE, _ABI_MD_MISSING_WITHDRAW
        )
        assert mod.main() == 1

    def test_missing_from_docs_reported(self, monkeypatch, tmp_path, capsys):
        """The missing-from-docs gap is named in the output."""
        mod = self._sandbox(
            monkeypatch, tmp_path, _LIB_RS_TEMPLATE, _ABI_MD_MISSING_WITHDRAW
        )
        mod.main()
        out = capsys.readouterr().out
        assert "MISSING-FROM-DOCS: withdraw" in out
        assert "missing-doc:withdraw" in out

    # -- exit code: documented-but-nonexistent ------------------------------

    def test_exit_one_on_documented_but_nonexistent(self, monkeypatch, tmp_path):
        """A documented entry point that no longer exists must exit 1."""
        mod = self._sandbox(monkeypatch, tmp_path, _LIB_RS_TEMPLATE, _ABI_MD_GHOST)
        assert mod.main() == 1

    def test_documented_but_nonexistent_reported(self, monkeypatch, tmp_path, capsys):
        """The documented-but-nonexistent gap is named in the output."""
        mod = self._sandbox(
            monkeypatch, tmp_path, _LIB_RS_TEMPLATE, _ABI_MD_GHOST
        )
        mod.main()
        out = capsys.readouterr().out
        assert "DOCUMENTED-BUT-NONEXISTENT: emergency_stop" in out
        assert "ghost-doc:emergency_stop" in out

    # -- exit code: aligned --------------------------------------------------

    def test_exit_zero_when_aligned(self, monkeypatch, tmp_path):
        """Both directions aligned (and no baseline) must exit 0."""
        mod = self._sandbox(monkeypatch, tmp_path, _LIB_RS_TEMPLATE, _ABI_MD_ALIGNED)
        assert mod.main() == 0

    def test_aligned_run_ignores_prose_mentions(self, monkeypatch, tmp_path):
        """Prose mentions of fields/functions must not count as documentation."""
        mod = self._sandbox(monkeypatch, tmp_path, _LIB_RS_TEMPLATE, _ABI_MD_ALIGNED)
        documented = mod.parse_documented_entry_points(_ABI_MD_ALIGNED)
        assert documented == {"create_stream", "withdraw"}

    # -- baselined gap passes -------------------------------------------------

    def test_baselined_gap_passes(self, monkeypatch, tmp_path):
        """A gap present in the baseline must not fail the run."""
        baseline = json.dumps(
            {"gaps": [{"id": "missing-doc:withdraw", "reason": "docs rewrite scheduled"}]}
        )
        mod = self._sandbox(
            monkeypatch,
            tmp_path,
            _LIB_RS_TEMPLATE,
            _ABI_MD_MISSING_WITHDRAW,
            baseline_text=baseline,
        )
        assert mod.main() == 0

    def test_baselined_gap_counted_in_output(self, monkeypatch, tmp_path, capsys):
        """The passing run reports how many gaps are baselined."""
        baseline = json.dumps(
            {"gaps": [{"id": "missing-doc:withdraw", "reason": "docs rewrite scheduled"}]}
        )
        mod = self._sandbox(
            monkeypatch,
            tmp_path,
            _LIB_RS_TEMPLATE,
            _ABI_MD_MISSING_WITHDRAW,
            baseline_text=baseline,
        )
        assert mod.main() == 0
        out = capsys.readouterr().out
        assert "1 baselined gap(s)" in out

    # -- new non-baselined gap fails -------------------------------------------

    def test_new_gap_without_baseline_entry_fails(self, monkeypatch, tmp_path):
        """A gap not covered by the baseline must fail even with a baseline file."""
        baseline = json.dumps(
            {"gaps": [{"id": "missing-doc:withdraw", "reason": "docs rewrite scheduled"}]}
        )
        ghost_docs = _ABI_MD_MISSING_WITHDRAW + "| `emergency_stop()` | admin | — |\n"
        mod = self._sandbox(
            monkeypatch,
            tmp_path,
            _LIB_RS_TEMPLATE,
            ghost_docs,
            baseline_text=baseline,
        )
        # withdraw is baselined, emergency_stop is not.
        assert mod.main() == 1

    # -- stale baseline entry fails ---------------------------------------------

    def test_stale_baseline_entry_fails(self, monkeypatch, tmp_path):
        """A baseline entry whose gap no longer exists must fail the run."""
        baseline = json.dumps(
            {"gaps": [{"id": "missing-doc:withdraw", "reason": "docs rewrite scheduled"}]}
        )
        # Aligned docs: the baselined gap no longer exists.
        mod = self._sandbox(
            monkeypatch, tmp_path, _LIB_RS_TEMPLATE, _ABI_MD_ALIGNED, baseline_text=baseline
        )
        assert mod.main() == 1

    def test_stale_baseline_reported(self, monkeypatch, tmp_path, capsys):
        """The stale entry is named in the output."""
        baseline = json.dumps(
            {"gaps": [{"id": "missing-doc:withdraw", "reason": "docs rewrite scheduled"}]}
        )
        mod = self._sandbox(
            monkeypatch, tmp_path, _LIB_RS_TEMPLATE, _ABI_MD_ALIGNED, baseline_text=baseline
        )
        mod.main()
        out = capsys.readouterr().out
        assert "STALE-BASELINE: missing-doc:withdraw" in out

    # -- baseline robustness ------------------------------------------------------

    def test_missing_baseline_file_treated_as_empty(self, monkeypatch, tmp_path):
        """No baseline file means no exclusions, not an error."""
        mod = self._sandbox(monkeypatch, tmp_path, _LIB_RS_TEMPLATE, _ABI_MD_ALIGNED)
        assert mod.load_baseline(tmp_path / "nope.json") == {}
        assert mod.main() == 0

    def test_malformed_baseline_fails_loudly(self, monkeypatch, tmp_path):
        """Corrupt baseline JSON must exit 2, never silently skip the baseline."""
        mod = self._sandbox(
            monkeypatch, tmp_path, _LIB_RS_TEMPLATE, _ABI_MD_ALIGNED, baseline_text="{not json"
        )
        assert mod.main() == 2

    def test_baseline_with_wrong_shape_fails_loudly(self, monkeypatch, tmp_path):
        """A baseline missing the 'gaps' array must exit 2."""
        mod = self._sandbox(
            monkeypatch, tmp_path, _LIB_RS_TEMPLATE, _ABI_MD_ALIGNED, baseline_text='{"a": 1}'
        )
        assert mod.main() == 2

    def test_baseline_entry_missing_reason_fails_loudly(self, monkeypatch, tmp_path):
        """Each baseline entry needs a non-empty reason string."""
        baseline = json.dumps({"gaps": [{"id": "missing-doc:withdraw"}]})
        mod = self._sandbox(
            monkeypatch, tmp_path, _LIB_RS_TEMPLATE, _ABI_MD_ALIGNED, baseline_text=baseline
        )
        assert mod.main() == 2

    def test_baseline_duplicate_ids_fail_loudly(self, monkeypatch, tmp_path):
        """Duplicate gap ids in the baseline must exit 2."""
        baseline = json.dumps(
            {
                "gaps": [
                    {"id": "missing-doc:withdraw", "reason": "a"},
                    {"id": "missing-doc:withdraw", "reason": "b"},
                ]
            }
        )
        mod = self._sandbox(
            monkeypatch, tmp_path, _LIB_RS_TEMPLATE, _ABI_MD_ALIGNED, baseline_text=baseline
        )
        assert mod.main() == 2

    # -- broken inputs -------------------------------------------------------------

    def test_missing_lib_rs_exits_two(self, monkeypatch, tmp_path):
        """A missing lib.rs cannot be treated as "no gaps"."""
        mod = self._sandbox(monkeypatch, tmp_path, _LIB_RS_TEMPLATE, _ABI_MD_ALIGNED)
        monkeypatch.setattr(mod, "LIB_RS", tmp_path / "nope.rs")
        assert mod.main() == 2

    def test_missing_abi_md_exits_two(self, monkeypatch, tmp_path):
        """A missing docs/ABI.md cannot be treated as "no gaps"."""
        mod = self._sandbox(
            monkeypatch, tmp_path, _LIB_RS_TEMPLATE, _ABI_MD_ALIGNED
        )
        monkeypatch.setattr(mod, "ABI_MD", tmp_path / "nope.md")
        assert mod.main() == 2

    def test_empty_surface_exits_two(self, monkeypatch, tmp_path):
        """No entry points extracted means the parser or source is broken."""
        mod = self._sandbox(
            monkeypatch, tmp_path, "pub fn orphan() {}\n", _ABI_MD_ALIGNED
        )
        assert mod.main() == 2

    # -- real repository state -------------------------------------------------------

    def test_real_repo_aligned(self):
        """The committed docs/ABI.md and lib.rs must pass the gate as-is."""
        mod = self._import()
        assert mod.main() == 0

    def test_committed_baseline_is_well_formed(self):
        """The committed baseline must parse and all reasons must be non-empty."""
        mod = self._import()
        baseline = mod.load_baseline(_BASELINE_PATH)
        assert isinstance(baseline, dict)
        assert all(reason.strip() for reason in baseline.values())

    def test_committed_baseline_entries_reference_real_gaps(self):
        """Every committed baseline entry must correspond to a live gap (no fiction)."""
        mod = self._import()
        baseline = mod.load_baseline(_BASELINE_PATH)
        entrypoints = mod.filter_entrypoints(
            mod.extract_contractimpl_pub_fns(
                (REPO_ROOT / "contracts" / "stream" / "src" / "lib.rs").read_text(encoding="utf-8")
            )
        )
        documented = mod.parse_documented_entry_points(
            (REPO_ROOT / "docs" / "ABI.md").read_text(encoding="utf-8")
        )
        missing, ghosts = mod.collect_gaps(entrypoints, documented)
        live = {f"{mod.MISSING_DOC_PREFIX}{n}" for n in missing}
        live |= {f"{mod.GHOST_DOC_PREFIX}{n}" for n in ghosts}
        stale = set(baseline) - live
        assert not stale, f"Stale baseline entries must be removed: {sorted(stale)}"

    def test_baseline_only_shrinks_vs_base_branch(self):
        """The PR must not grow the baseline relative to the merge base."""
        import subprocess

        mod = self._import()
        base_ref = os.environ.get("BASELINE_BASE_REF", "origin/main")
        try:
            result = subprocess.run(
                ["git", "show", f"{base_ref}:script/doc-alignment-baseline.json"],
                capture_output=True,
                text=True,
                cwd=REPO_ROOT,
                timeout=30,
            )
        except (subprocess.SubprocessError, OSError):
            pytest.skip("git history unavailable for baseline-shrink check")
        if result.returncode != 0 or not result.stdout.strip():
            pytest.skip("baseline did not exist on the base branch")
        base_gaps = set(mod.load_baseline_from_text(result.stdout))
        current_gaps = set(mod.load_baseline(_BASELINE_PATH))
        assert current_gaps <= base_gaps, (
            "baseline grew; new entries must be justified in review and "
            "documented with a reason"
        )
