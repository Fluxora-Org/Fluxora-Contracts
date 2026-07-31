"""Regression guards for the upgrade runbook and exported upgrade surface."""

from pathlib import Path
import re


ROOT = Path(__file__).resolve().parents[1]
LIB = (ROOT / "contracts/stream/src/lib.rs").read_text(encoding="utf-8")
DOC = (ROOT / "docs/upgrade.md").read_text(encoding="utf-8")
UPGRADE_TESTS = (
    ROOT / "contracts/stream/tests/upgrade_path.rs"
).read_text(encoding="utf-8")
STORAGE_TESTS = (
    ROOT / "contracts/stream/tests/storage_key_compat.rs"
).read_text(encoding="utf-8")
CHECKSUM_UPDATE = (ROOT / "script/update-wasm-checksums.sh").read_text(encoding="utf-8")
CHECKSUM_VERIFY = (ROOT / "script/verify-wasm-checksum.sh").read_text(encoding="utf-8")


def test_upgrade_is_exported_and_exercised_through_generated_client():
    # There is an associated ABI method and a module-level compatibility helper.
    # The Rust integration test compiling `try_upgrade` is the generated-client guard.
    assert len(
        re.findall(r"pub fn upgrade\s*\(\s*env: Env,\s*new_wasm_hash:", LIB)
    ) == 2
    assert "client.try_upgrade(&new_hash)" in UPGRADE_TESTS
    assert "test_upgrade_fails_if_not_initialized" in UPGRADE_TESTS


def test_version_and_current_storage_boundary_stay_in_sync():
    version = int(
        re.search(r"pub const CONTRACT_VERSION:\s*u32\s*=\s*(\d+)", LIB).group(1)
    )
    assert version == 9
    assert "CONTRACT_VERSION = 9" in DOC
    assert "37 `DataKey` variants (0..=36)" in DOC
    assert "DataKey::DelegatedCancelNonce(dummy_addr.clone()),     // 36" in STORAGE_TESTS
    assert "DataKey::DelegatedCancelNonce(_) => {}" in STORAGE_TESTS
    assert "expected_datakey_count_for_version(9), 37" in STORAGE_TESTS


def test_event_version_semantics_do_not_claim_target_introspection():
    assert "continues running the old WASM after" in DOC
    assert "legacy.old_version == legacy.new_version == executing CONTRACT_VERSION" in DOC
    assert "They are **not** an introspection of the replacement hash" in DOC
    assert "call `version()` after" in DOC
    assert "new_version: executing_version" in LIB


def test_resource_and_ttl_guidance_matches_implementation():
    assert "Uploading the WASM and invoking `upgrade()` are separate" in DOC
    assert "Do not use a hard-coded \"gas units\" estimate" in DOC
    assert "60,000–225,000" not in DOC
    assert "does **not** enumerate or extend persistent entries" in DOC
    assert "There is no `stellar contract bump --all` guarantee" in DOC
    assert "--key-xdr \"$KEY_XDR\"" in DOC


def test_atomic_failure_and_recovery_limits_are_explicit():
    for test_name in (
        "test_failed_upgrade_rolls_back_events_and_config",
        "test_version_stable_after_failed_upgrade",
        "test_contract_usable_after_failed_upgrade",
        "test_admin_rotation_possible_after_failed_upgrade",
        "test_version_budget_is_lower_than_storage_backed_view",
    ):
        assert test_name in UPGRADE_TESTS
        assert test_name in DOC

    assert "There is no partially upgraded state" in DOC
    assert "currently running replacement still exposes a working `upgrade`" in DOC
    assert "there is no automatic rollback or privileged bypass" in DOC


def test_in_place_and_new_instance_migrations_are_distinguished():
    assert "## 3. New-Instance Migration Runbook" in DOC
    assert "## 6. In-Place Contract Upgrades" in DOC
    assert "does **not** by itself require a new instance" in DOC
    assert "no on-chain state-copy path exists between contract IDs" in DOC


def test_checksum_builds_do_not_hash_import_only_stream_artifact():
    expected_order = "CONTRACT_ORDER=(fluxora_factory fluxora_governance fluxora_stream)"
    assert expected_order in CHECKSUM_UPDATE
    assert 'for name in "${CONTRACT_ORDER[@]}"' in CHECKSUM_UPDATE
    assert "for package in fluxora_factory fluxora_governance fluxora_stream" in CHECKSUM_VERIFY
    assert "-p fluxora_stream \\\n    -p fluxora_factory" not in CHECKSUM_VERIFY
