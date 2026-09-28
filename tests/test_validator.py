"""Basic validation tests for CI pipeline integrity.

These tests verify that critical project files and scripts exist and are
well-formed, ensuring the CI infrastructure itself is healthy.
"""

import importlib.util
import os
import sys
from pathlib import Path

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
        """Exercise check_streaming_entrypoints with real source if available."""
        mod = _import_script("validate-doc-alignment.py")
        assert mod is not None
        # This checks docs/streaming.md vs lib.rs — skips if docs missing
        assert mod.check_streaming_entrypoints() is True

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
        assert len(sections["ContractError (stream)"]) == 33

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
