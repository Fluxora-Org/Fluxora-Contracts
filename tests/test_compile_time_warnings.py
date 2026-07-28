"""Regression tests for the stream contract's compile-time warning policy.

These tests intentionally inspect source/configuration: the behavior under test is
which declaration owns compile-time symbols and whether CI treats diagnostics as
errors. Runtime contract behavior is covered by the Rust contract suites.
"""

from pathlib import Path
import re


ROOT = Path(__file__).resolve().parents[1]
LIB = (ROOT / "contracts/stream/src/lib.rs").read_text()
STORAGE = (ROOT / "contracts/stream/src/storage.rs").read_text()
CI = (ROOT / ".github/workflows/ci.yml").read_text()

# Helpers moved out of lib.rs must stay single-owned by storage.rs. Shadow copies
# generate hidden_glob_reexports warnings and can drift behaviorally.
CANONICAL_STORAGE_HELPERS = (
    "acquire_reentrancy_lock",
    "release_reentrancy_lock",
    "compute_adaptive_ttl",
    "get_config",
    "load_config",
    "get_token",
    "get_admin",
    "is_global_emergency_paused",
    "is_creation_paused",
    "require_not_globally_paused",
    "require_not_creation_paused",
    "is_protocol_paused",
    "get_pause_reason",
    "get_pause_timestamp",
    "get_pause_admin",
    "get_max_rate_per_second",
    "set_max_rate_per_second",
    "read_stream_count",
    "set_stream_count",
    "read_paused_stream_count",
    "write_paused_stream_count",
    "reconcile_paused_stream_count",
    "next_stream_id_for",
    "read_pooled_stream_shares",
    "save_pooled_stream_shares",
    "read_pooled_stream_withdrawn",
    "save_pooled_stream_withdrawn",
    "reject_duplicate_ids",
)


def top_level_function_definition_count(source: str, name: str) -> int:
    """Count free functions only (contract entrypoints are indented in an impl)."""
    return len(re.findall(rf"(?m)^(?:pub(?:\([^)]*\))?\s+)?fn\s+{name}\s*\(", source))


def named_block(source: str, declaration: str) -> str:
    """Return a simple Rust brace block beginning at ``declaration``."""
    start = source.index(declaration)
    brace = source.index("{", start)
    depth = 0
    for index in range(brace, len(source)):
        if source[index] == "{":
            depth += 1
        elif source[index] == "}":
            depth -= 1
            if depth == 0:
                return source[start : index + 1]
    raise AssertionError(f"unterminated declaration: {declaration}")


def test_storage_helpers_have_one_canonical_owner():
    for helper in CANONICAL_STORAGE_HELPERS:
        assert top_level_function_definition_count(STORAGE, helper) == 1, helper
        assert top_level_function_definition_count(LIB, helper) == 0, helper

    assert "pub use storage::*;" in LIB


def test_orphan_type_module_cannot_reintroduce_duplicate_contract_types():
    assert "mod types;" not in LIB
    assert not (ROOT / "contracts/stream/src/types.rs").exists()

    # The contract macro exports these names; a duplicate is a compile/spec error.
    for declaration in (
        "pub struct Stream {",
        "pub struct ClaimOwnershipTransferred {",
        "pub struct RecipientShareDelegated {",
        "pub const MAX_POOL_RECIPIENTS:",
    ):
        assert LIB.count(declaration) == 1, declaration


def test_compatibility_sensitive_fields_are_unique_and_stable():
    stream = named_block(LIB, "pub struct Stream {")
    fields = re.findall(r"(?m)^\s*pub\s+(\w+)\s*:", stream)
    assert len(fields) == len(set(fields)), "Stream contains duplicate fields"
    assert fields == [
        "stream_id",
        "sender",
        "recipient",
        "claim_owner",
        "deposit_amount",
        "rate_per_second",
        "start_time",
        "cliff_time",
        "end_time",
        "withdrawn_amount",
        "status",
        "cancelled_at",
        "checkpointed_amount",
        "checkpointed_at",
        "withdraw_dust_threshold",
        "last_pause_toggle_ledger",
        "last_withdraw_ledger",
        "last_rate_change_ledger",
        "is_pooled",
        "metadata",
        "memo",
        "kind",
        "irrevocable",
        "witness",
        "delegation_depth",
        "parent_stream_id",
        "decommissioned",
    ]

    errors = named_block(LIB, "pub enum ContractError {")
    variants = re.findall(r"(?m)^\s*(\w+)\s*=\s*(\d+),", errors)
    names = [name for name, _ in variants]
    assert len(names) == len(set(names)), "ContractError contains duplicate variants"
    assert ("ReservationAlreadyActive", "34") in variants
    assert ("OfferWrongSender", "41") in variants

    assert re.search(r"pub const CONTRACT_VERSION:\s*u32\s*=\s*9\s*;", LIB)


def test_warning_suppression_is_narrow_and_explained():
    crate_attributes = "\n".join(LIB.splitlines()[:12])
    assert "clippy::too_many_arguments" in crate_attributes
    assert "stable public ABI" in crate_attributes

    forbidden = (
        "#![allow(warnings)]",
        "#![allow(unused)]",
        "#![allow(unused_imports)]",
        "#![allow(dead_code)]",
    )
    for attribute in forbidden:
        assert attribute not in LIB


def test_stream_warning_gate_is_hard_in_ci():
    command = "cargo clippy -p fluxora_stream --lib -- -D warnings"
    assert CI.count(command) == 1

    # The step ends at the next equally-indented step. It must not opt out.
    command_start = CI.index(command)
    next_step = CI.find("\n      - name:", command_start)
    step = CI[command_start : next_step if next_step != -1 else len(CI)]
    assert "continue-on-error" not in step


def test_documented_regression_surface_covers_required_edges():
    policy = (ROOT / "docs/compile-time-warnings.md").read_text().lower()
    for topic in (
        "current behavior",
        "cleanup behavior",
        "storage and state",
        "gas and wasm size",
        "upgrade and compatibility",
        "expected regression surface",
        "test/testutils builds",
    ):
        assert topic in policy
