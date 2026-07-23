"""
tests/test_validator.py

Test suite for script/validate-doc-alignment.py.
Uses pytest and monkeypatch to simulate file-system states.
Targets 95%+ code coverage of the validator module.
"""

import importlib.util
from pathlib import Path

import pytest

# ---------------------------------------------------------------------------
# Load the module under test without executing __main__
# ---------------------------------------------------------------------------

_SCRIPT = Path(__file__).resolve().parent.parent / "script" / "validate-doc-alignment.py"


def _load_module():
    spec = importlib.util.spec_from_file_location("validate_doc_alignment", _SCRIPT)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


vda = _load_module()

# ---------------------------------------------------------------------------
# Shared source / doc stubs
# ---------------------------------------------------------------------------

MINIMAL_LIB_RS = """\
#[contractimpl]
impl MyContract {
    pub fn init(env: Env) -> Result<(), Error> { Ok(()) }
    pub fn create_stream(env: Env) -> Result<u64, Error> { Ok(0) }
    pub fn withdraw(env: Env) -> Result<i128, Error> { Ok(0) }
}
pub fn save_stream(env: &Env) {}
fn private_helper() {}
"""

MINIMAL_EVENTS_RS = """\
pub fn emit_created(env: &Env, id: u64) {
    env.events().publish(
        (Symbol::short(&env, "created"), id),
        payload,
    );
}
pub fn emit_withdrew(env: &Env, id: u64) {
    env.events().publish(
        (Symbol::new(&env, "withdrew"), id),
        payload,
    );
}
"""

MINIMAL_ERROR_RS = """\
#[contracterror]
pub enum ContractError {
    StreamNotFound = 1,
    InvalidState = 2,
}
"""

STREAMING_DOC = "# Streaming\n`init`, `create_stream`, `withdraw` are entrypoints.\n"
EVENTS_DOC = "# Events\n`created` and `withdrew` are the event topics.\n"
ERROR_DOC = "# Errors\n`StreamNotFound` = 1, `InvalidState` = 2.\n"


def _write_files(
    tmp_path: Path,
    lib_rs: str = MINIMAL_LIB_RS,
    events_rs: str = MINIMAL_EVENTS_RS,
    error_rs: str = MINIMAL_ERROR_RS,
    streaming: str = STREAMING_DOC,
    events: str = EVENTS_DOC,
    error: str = ERROR_DOC,
):
    """Write all six files to tmp_path and return their paths as a tuple."""
    data = {
        "lib.rs": lib_rs,
        "events.rs": events_rs,
        "error.rs": error_rs,
        "streaming.md": streaming,
        "events.md": events,
        "error.md": error,
    }
    paths = {}
    for name, content in data.items():
        p = tmp_path / name
        p.write_text(content, encoding="utf-8")
        paths[name] = p
    return (
        paths["lib.rs"],
        paths["events.rs"],
        paths["error.rs"],
        paths["streaming.md"],
        paths["events.md"],
        paths["error.md"],
    )


def _fake_mapping(tmp_path: Path, files: tuple, missing_key: str = None) -> dict:
    """Build a MAPPING dict pointing at real tmp_path files."""
    keys = ["CONTRACT_SRC", "EVENTS_SRC", "ERROR_SRC",
            "DOC_STREAMING", "DOC_EVENTS", "DOC_ERROR"]
    names = ["lib.rs", "events.rs", "error.rs",
             "streaming.md", "events.md", "error.md"]
    mapping = {}
    for key, name, path in zip(keys, names, files):
        if key == missing_key:
            mapping[key] = (tmp_path / "no_such_file_xyz.rs",
                            "**/no_such_file_xyz_unique.rs")
        else:
            mapping[key] = (path, f"**/{name}")
    return mapping


# ---------------------------------------------------------------------------
# resolve_path
# ---------------------------------------------------------------------------

class TestResolvePath:
    def test_returns_canonical_when_exists(self, tmp_path):
        f = tmp_path / "lib.rs"
        f.write_text("", encoding="utf-8")
        assert vda.resolve_path("X", f, "**/*.rs") == f

    def test_falls_back_to_glob(self, tmp_path):
        sub = tmp_path / "a" / "b"
        sub.mkdir(parents=True)
        target = sub / "lib.rs"
        target.write_text("", encoding="utf-8")
        orig = vda.REPO_ROOT
        vda.REPO_ROOT = tmp_path
        try:
            result = vda.resolve_path("X", tmp_path / "missing.rs", "**/lib.rs")
        finally:
            vda.REPO_ROOT = orig
        assert result == target

    def test_returns_none_when_both_miss(self, tmp_path):
        orig = vda.REPO_ROOT
        vda.REPO_ROOT = tmp_path
        try:
            result = vda.resolve_path("X", tmp_path / "nope.rs", "**/nope_xyz.rs")
        finally:
            vda.REPO_ROOT = orig
        assert result is None

    def test_glob_returns_first_sorted_match(self, tmp_path):
        for name in ("b_lib.rs", "a_lib.rs"):
            (tmp_path / name).write_text("", encoding="utf-8")
        orig = vda.REPO_ROOT
        vda.REPO_ROOT = tmp_path
        try:
            result = vda.resolve_path("X", tmp_path / "missing.rs", "**/*_lib.rs")
        finally:
            vda.REPO_ROOT = orig
        assert result is not None
        assert result.name == "a_lib.rs"


# ---------------------------------------------------------------------------
# resolve_all
# ---------------------------------------------------------------------------

class TestResolveAll:
    def test_all_present_returns_ok(self, tmp_path, monkeypatch):
        files = _write_files(tmp_path)
        monkeypatch.setattr(vda, "MAPPING", _fake_mapping(tmp_path, files))
        monkeypatch.setattr(vda, "REPO_ROOT", tmp_path)
        resolved, ok = vda.resolve_all()
        assert ok is True
        assert len(resolved) == 6

    def test_missing_file_returns_not_ok(self, tmp_path, monkeypatch):
        files = _write_files(tmp_path)
        monkeypatch.setattr(vda, "MAPPING",
                            _fake_mapping(tmp_path, files, "CONTRACT_SRC"))
        monkeypatch.setattr(vda, "REPO_ROOT", tmp_path)
        _, ok = vda.resolve_all()
        assert ok is False

    def test_missing_prints_file_missing_tag(self, tmp_path, monkeypatch, capsys):
        files = _write_files(tmp_path)
        monkeypatch.setattr(vda, "MAPPING",
                            _fake_mapping(tmp_path, files, "EVENTS_SRC"))
        monkeypatch.setattr(vda, "REPO_ROOT", tmp_path)
        vda.resolve_all()
        assert "[FILE MISSING]:" in capsys.readouterr().out

    def test_missing_prints_debug_tree(self, tmp_path, monkeypatch, capsys):
        files = _write_files(tmp_path)
        monkeypatch.setattr(vda, "MAPPING",
                            _fake_mapping(tmp_path, files, "DOC_ERROR"))
        monkeypatch.setattr(vda, "REPO_ROOT", tmp_path)
        vda.resolve_all()
        out = capsys.readouterr().out
        assert "[CWD]" in out
        assert "[ROOT]" in out

    def test_no_debug_tree_when_all_present(self, tmp_path, monkeypatch, capsys):
        files = _write_files(tmp_path)
        monkeypatch.setattr(vda, "MAPPING", _fake_mapping(tmp_path, files))
        monkeypatch.setattr(vda, "REPO_ROOT", tmp_path)
        vda.resolve_all()
        assert "[CWD]" not in capsys.readouterr().out

    def test_resolved_excludes_missing_key(self, tmp_path, monkeypatch):
        files = _write_files(tmp_path)
        monkeypatch.setattr(vda, "MAPPING",
                            _fake_mapping(tmp_path, files, "ERROR_SRC"))
        monkeypatch.setattr(vda, "REPO_ROOT", tmp_path)
        resolved, _ = vda.resolve_all()
        assert "ERROR_SRC" not in resolved

    def test_missing_message_contains_path(self, tmp_path, monkeypatch, capsys):
        files = _write_files(tmp_path)
        monkeypatch.setattr(vda, "MAPPING",
                            _fake_mapping(tmp_path, files, "CONTRACT_SRC"))
        monkeypatch.setattr(vda, "REPO_ROOT", tmp_path)
        vda.resolve_all()
        assert "no_such_file_xyz.rs" in capsys.readouterr().out


# ---------------------------------------------------------------------------
# _print_debug_tree
# ---------------------------------------------------------------------------

class TestPrintDebugTree:
    def test_prints_cwd_and_root(self, tmp_path, capsys):
        vda._print_debug_tree(tmp_path)
        out = capsys.readouterr().out
        assert "[CWD]" in out
        assert "[ROOT]" in out

    def test_lists_files(self, tmp_path, capsys):
        (tmp_path / "myfile.txt").write_text("x", encoding="utf-8")
        vda._print_debug_tree(tmp_path)
        assert "myfile.txt" in capsys.readouterr().out

    def test_respects_max_depth(self, tmp_path, capsys):
        deep = tmp_path / "a" / "b" / "c" / "d" / "e"
        deep.mkdir(parents=True)
        (deep / "deep.txt").write_text("x", encoding="utf-8")
        vda._print_debug_tree(tmp_path, max_depth=2)
        assert "deep.txt" not in capsys.readouterr().out

    def test_directories_marked_with_slash(self, tmp_path, capsys):
        (tmp_path / "subdir").mkdir()
        vda._print_debug_tree(tmp_path)
        assert "subdir/" in capsys.readouterr().out


# ---------------------------------------------------------------------------
# extract_entrypoints
# ---------------------------------------------------------------------------

class TestExtractEntrypoints:
    def test_finds_pub_fn(self):
        assert "init" in vda.extract_entrypoints("pub fn init(env: Env) {}")

    def test_ignores_private_fn(self):
        assert "helper" not in vda.extract_entrypoints("fn helper() {}")

    def test_allowlist_excluded(self):
        assert "save_stream" not in vda.extract_entrypoints(
            "pub fn save_stream(env: &Env) {}")

    def test_multiple_entrypoints(self):
        src = "pub fn alpha() {}\npub fn beta() {}"
        assert {"alpha", "beta"}.issubset(vda.extract_entrypoints(src))

    def test_indented_pub_fn(self):
        assert "indented" in vda.extract_entrypoints(
            "    pub fn indented(env: Env) {}")

    def test_generic_pub_fn(self):
        assert "generic_fn" in vda.extract_entrypoints(
            "pub fn generic_fn<T>(x: T) {}")

    def test_empty_source(self):
        assert vda.extract_entrypoints("") == set()

    def test_returns_set_type(self):
        assert isinstance(vda.extract_entrypoints("pub fn foo() {}"), set)


# ---------------------------------------------------------------------------
# extract_event_symbols
# ---------------------------------------------------------------------------

class TestExtractEventSymbols:
    def test_finds_symbol_short(self):
        assert "created" in vda.extract_event_symbols(
            'Symbol::short(&env, "created")')

    def test_finds_symbol_new(self):
        assert "withdrew" in vda.extract_event_symbols(
            'Symbol::new(&env, "withdrew")')

    def test_finds_both_variants(self):
        src = 'Symbol::short(&env, "paused") Symbol::new(&env, "resumed")'
        assert {"paused", "resumed"}.issubset(vda.extract_event_symbols(src))

    def test_deduplicates(self):
        src = 'Symbol::short(&env, "x") Symbol::short(&env, "x")'
        assert len(vda.extract_event_symbols(src)) == 1

    def test_whitespace_tolerance(self):
        assert "spaced" in vda.extract_event_symbols(
            'Symbol::short( &env , "spaced" )')

    def test_matches_symbol_short_macro(self):
        assert {"old_style"} == vda.extract_event_symbols(
            'symbol_short!("old_style")'
        )

    def test_empty_source(self):
        assert vda.extract_event_symbols("") == set()

    def test_returns_set_type(self):
        assert isinstance(
            vda.extract_event_symbols('Symbol::short(&e, "x")'), set)


# ---------------------------------------------------------------------------
# extract_error_variants
# ---------------------------------------------------------------------------

class TestExtractErrorVariants:
    def test_finds_variants(self):
        src = "    StreamNotFound = 1,\n    InvalidState = 2,"
        assert {"StreamNotFound", "InvalidState"} == vda.extract_error_variants(src)

    def test_ignores_lowercase_names(self):
        src = "    notAVariant = 1,\n    ValidVariant = 2,"
        result = vda.extract_error_variants(src)
        assert "ValidVariant" in result
        assert "notAVariant" not in result

    def test_no_variants(self):
        assert vda.extract_error_variants("no enum here") == set()

    def test_empty_source(self):
        assert vda.extract_error_variants("") == set()

    def test_returns_set_type(self):
        assert isinstance(vda.extract_error_variants("    Foo = 1,"), set)

    def test_multiple_variants(self):
        src = "    Alpha = 1,\n    Beta = 2,\n    Gamma = 3,"
        assert vda.extract_error_variants(src) == {"Alpha", "Beta", "Gamma"}


# ---------------------------------------------------------------------------
# check_missing
# ---------------------------------------------------------------------------

class TestCheckMissing:
    def test_all_present(self):
        assert vda.check_missing({"foo", "bar"}, "foo bar baz") == set()

    def test_some_missing(self):
        assert vda.check_missing(
            {"foo", "xyz_absent"}, "foo is here") == {"xyz_absent"}

    def test_all_missing(self):
        assert vda.check_missing(
            {"xyz_foo", "xyz_bar"}, "nothing") == {"xyz_foo", "xyz_bar"}

    def test_empty_identifiers(self):
        assert vda.check_missing(set(), "anything") == set()

    def test_empty_doc(self):
        assert vda.check_missing({"foo"}, "") == {"foo"}

    def test_returns_set_type(self):
        assert isinstance(vda.check_missing({"a"}, "a"), set)


# ---------------------------------------------------------------------------
# validate()
# ---------------------------------------------------------------------------

class TestValidate:
    def test_passes_on_full_alignment(self, tmp_path):
        assert vda.validate(*_write_files(tmp_path)) == 0

    def test_fails_on_missing_entrypoint(self, tmp_path):
        paths = _write_files(
            tmp_path,
            streaming="# Streaming\nOnly `init` is documented here.\n")
        assert vda.validate(*paths) == 1

    def test_fails_on_missing_event_symbol(self, tmp_path):
        paths = _write_files(
            tmp_path,
            events="# Events\nOnly `created` is documented here.\n")
        assert vda.validate(*paths) == 1

    def test_fails_on_missing_error_variant(self, tmp_path):
        paths = _write_files(
            tmp_path,
            error="# Errors\nOnly `StreamNotFound` is documented here.\n")
        assert vda.validate(*paths) == 1

    def test_fails_on_all_docs_drifted(self, tmp_path):
        paths = _write_files(
            tmp_path,
            streaming="# Streaming\nno entrypoints\n",
            events="# Events\nno symbols\n",
            error="# Errors\nno variants\n",
        )
        assert vda.validate(*paths) == 1

    def test_allowlisted_entrypoint_not_required(self, tmp_path):
        paths = _write_files(
            tmp_path,
            streaming="# Streaming\n`init`, `create_stream`, `withdraw`\n")
        assert vda.validate(*paths) == 0

    def test_prints_ok_on_success(self, tmp_path, capsys):
        vda.validate(*_write_files(tmp_path))
        assert "OK:" in capsys.readouterr().out

    def test_prints_missing_doc_message(self, tmp_path, capsys):
        paths = _write_files(
            tmp_path,
            streaming="# Streaming\nOnly `init` is documented here.\n")
        vda.validate(*paths)
        out = capsys.readouterr().out
        assert "MISSING DOC:" in out
        assert "streaming.md" in out

    def test_missing_entrypoint_message_contains_kind(self, tmp_path, capsys):
        paths = _write_files(
            tmp_path,
            streaming="# Streaming\nOnly `init` is documented here.\n")
        vda.validate(*paths)
        assert "entrypoint" in capsys.readouterr().out

    def test_missing_event_message_contains_kind(self, tmp_path, capsys):
        paths = _write_files(
            tmp_path,
            events="# Events\nOnly `created` is documented here.\n")
        vda.validate(*paths)
        assert "event symbol" in capsys.readouterr().out

    def test_missing_error_message_contains_kind(self, tmp_path, capsys):
        paths = _write_files(
            tmp_path,
            error="# Errors\nOnly `StreamNotFound` is documented here.\n")
        vda.validate(*paths)
        assert "error variant" in capsys.readouterr().out

    def test_utf8_encoding_roundtrip(self, tmp_path):
        paths = _write_files(
            tmp_path,
            streaming="# Streaming\n`init`, `create_stream`, `withdraw` — résumé\n")
        assert vda.validate(*paths) == 0

    def test_path_outside_repo_root_does_not_raise(self, tmp_path, capsys):
        # tmp_path is outside REPO_ROOT; relative_to raises ValueError which
        # the code handles gracefully by falling back to the full path.
        paths = _write_files(
            tmp_path,
            streaming="# Streaming\nOnly `init` is documented here.\n")
        vda.validate(*paths)
        assert "MISSING DOC:" in capsys.readouterr().out


# ---------------------------------------------------------------------------
# main()
# ---------------------------------------------------------------------------

class TestMain:
    def _patch(self, monkeypatch, tmp_path, missing_key=None):
        """Patch vda.MAPPING to point at tmp_path files, optionally making one missing."""
        files = _write_files(tmp_path)
        monkeypatch.setattr(vda, "MAPPING",
                            _fake_mapping(tmp_path, files, missing_key))
        monkeypatch.setattr(vda, "REPO_ROOT", tmp_path)

    def test_all_aligned_returns_0(self, tmp_path, monkeypatch):
        self._patch(monkeypatch, tmp_path)
        assert vda.main() == 0

    def test_missing_contract_returns_1(self, tmp_path, monkeypatch):
        self._patch(monkeypatch, tmp_path, "CONTRACT_SRC")
        assert vda.main() == 1

    def test_missing_events_src_returns_1(self, tmp_path, monkeypatch):
        self._patch(monkeypatch, tmp_path, "EVENTS_SRC")
        assert vda.main() == 1

    def test_missing_error_src_returns_1(self, tmp_path, monkeypatch):
        self._patch(monkeypatch, tmp_path, "ERROR_SRC")
        assert vda.main() == 1

    def test_missing_streaming_doc_returns_1(self, tmp_path, monkeypatch):
        self._patch(monkeypatch, tmp_path, "DOC_STREAMING")
        assert vda.main() == 1

    def test_missing_events_doc_returns_1(self, tmp_path, monkeypatch):
        self._patch(monkeypatch, tmp_path, "DOC_EVENTS")
        assert vda.main() == 1

    def test_missing_error_doc_returns_1(self, tmp_path, monkeypatch):
        self._patch(monkeypatch, tmp_path, "DOC_ERROR")
        assert vda.main() == 1

    def test_missing_file_prints_file_missing_tag(
            self, tmp_path, monkeypatch, capsys):
        self._patch(monkeypatch, tmp_path, "CONTRACT_SRC")
        vda.main()
        assert "[FILE MISSING]:" in capsys.readouterr().out

    def test_drift_returns_1_via_main(self, tmp_path, monkeypatch):
        files = _write_files(
            tmp_path,
            streaming="# Streaming\nOnly `init` is documented here.\n")
        monkeypatch.setattr(vda, "MAPPING", _fake_mapping(tmp_path, files))
        monkeypatch.setattr(vda, "REPO_ROOT", tmp_path)
        assert vda.main() == 1


# ---------------------------------------------------------------------------
# Additional coverage
# ---------------------------------------------------------------------------

class TestExtractErrorVariantsExcludeList:
    """ERROR_EXTRACT_EXCLUDE variants must be silently dropped."""

    def test_excluded_variants_not_returned(self):
        # All names in ERROR_EXTRACT_EXCLUDE should be filtered out even
        # when they match the CamelCase = <int> pattern.
        src = (
            "    Operational = 1,\n"
            "    Administrative = 2,\n"
            "    Compliance = 3,\n"
            "    Emergency = 4,\n"
            "    GlobalEmergency = 5,\n"
        )
        assert vda.extract_error_variants(src) == set()

    def test_excluded_and_real_variants_mixed(self):
        # Real variants survive; excluded ones are stripped.
        src = (
            "    Operational = 1,\n"
            "    StreamNotFound = 2,\n"
            "    Emergency = 3,\n"
            "    GlobalEmergency = 4,\n"
            "    InvalidState = 5,\n"
        )
        result = vda.extract_error_variants(src)
        assert result == {"StreamNotFound", "InvalidState"}


class TestEntrypointAllowlistFullCoverage:
    """Every name in ENTRYPOINT_ALLOWLIST must be suppressed."""

    def test_require_not_paused_excluded(self):
        assert "require_not_paused" not in vda.extract_entrypoints(
            "pub fn require_not_paused(env: &Env) {}"
        )

    def test_require_not_globally_paused_excluded(self):
        assert "require_not_globally_paused" not in vda.extract_entrypoints(
            "pub fn require_not_globally_paused(env: &Env) {}"
        )


# ---------------------------------------------------------------------------
# validate_gas.py tests
# ---------------------------------------------------------------------------

import importlib.util
import json
import textwrap
import types
import unittest.mock as mock
from pathlib import Path

_GAS_SCRIPT = Path(__file__).resolve().parent.parent / "script" / "validate_gas.py"


def _load_gas_module():
    spec = importlib.util.spec_from_file_location("validate_gas", _GAS_SCRIPT)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


vg = _load_gas_module()

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------

_BASELINE_BLOCK = textwrap.dedent("""\
    <!-- GAS_BASELINE_START -->
    {
        "create_stream": 1000,
        "withdraw": 500,
        "batch_withdraw": {"small": 200, "large": 800}
    }
    <!-- GAS_BASELINE_END -->
""")

_GAS_MD_NO_BLOCK = "# Gas\nNo baseline here.\n"


# ---------------------------------------------------------------------------
# extract_baselines
# ---------------------------------------------------------------------------

class TestExtractBaselines:
    def test_parses_valid_block(self, tmp_path):
        f = tmp_path / "gas.md"
        f.write_text(_BASELINE_BLOCK, encoding="utf-8")
        result = vg.extract_baselines(str(f))
        assert result["create_stream"] == 1000
        assert result["withdraw"] == 500
        assert result["batch_withdraw"]["small"] == 200

    def test_raises_on_missing_block(self, tmp_path):
        f = tmp_path / "gas.md"
        f.write_text(_GAS_MD_NO_BLOCK, encoding="utf-8")
        with pytest.raises(ValueError, match="Could not find gas baseline"):
            vg.extract_baselines(str(f))

    def test_returns_dict(self, tmp_path):
        f = tmp_path / "gas.md"
        f.write_text(_BASELINE_BLOCK, encoding="utf-8")
        assert isinstance(vg.extract_baselines(str(f)), dict)


# ---------------------------------------------------------------------------
# parse_measurements
# ---------------------------------------------------------------------------

class TestParseMeasurements:
    def test_parses_single_measurement(self):
        output = "GAS_MEASUREMENT: create_stream: single: 1050\n"
        result = vg.parse_measurements(output)
        assert result == {"create_stream": {"single": 1050}}

    def test_parses_multiple_functions(self):
        output = (
            "GAS_MEASUREMENT: create_stream: single: 1050\n"
            "GAS_MEASUREMENT: withdraw: single: 510\n"
        )
        result = vg.parse_measurements(output)
        assert "create_stream" in result
        assert "withdraw" in result

    def test_parses_batch_sizes(self):
        output = (
            "GAS_MEASUREMENT: batch_withdraw: small: 210\n"
            "GAS_MEASUREMENT: batch_withdraw: large: 820\n"
        )
        result = vg.parse_measurements(output)
        assert result["batch_withdraw"]["small"] == 210
        assert result["batch_withdraw"]["large"] == 820

    def test_ignores_non_matching_lines(self):
        output = "INFO: something else\nno match here\n"
        assert vg.parse_measurements(output) == {}

    def test_empty_output(self):
        assert vg.parse_measurements("") == {}

    def test_returns_dict(self):
        assert isinstance(vg.parse_measurements(""), dict)


# ---------------------------------------------------------------------------
# run_tests
# ---------------------------------------------------------------------------

class TestRunTests:
    def test_returns_string(self, monkeypatch):
        fake_result = types.SimpleNamespace(stdout="GAS_MEASUREMENT: x: single: 1\n")
        monkeypatch.setattr(
            vg.subprocess, "run", lambda *a, **kw: fake_result
        )
        output = vg.run_tests()
        assert isinstance(output, str)
        assert "GAS_MEASUREMENT" in output

    def test_passes_nocapture_flag(self, monkeypatch):
        captured = {}

        def fake_run(cmd, **kwargs):
            captured["cmd"] = cmd
            return types.SimpleNamespace(stdout="")

        monkeypatch.setattr(vg.subprocess, "run", fake_run)
        vg.run_tests()
        assert "--nocapture" in captured["cmd"]

    def test_ignores_nonzero_returncode(self, monkeypatch):
        fake_result = types.SimpleNamespace(stdout="some output")
        monkeypatch.setattr(
            vg.subprocess, "run", lambda *a, **kw: fake_result
        )
        # Should not raise even if cargo fails
        result = vg.run_tests()
        assert result == "some output"


# ---------------------------------------------------------------------------
# main — pass / fail / error paths
# ---------------------------------------------------------------------------

class TestGasMain:
    def _patch(self, monkeypatch, tmp_path, measurements, baseline_override=None):
        """Wire up main() with a temp gas.md and fake subprocess output."""
        gas_md = tmp_path / "gas.md"
        gas_md.write_text(_BASELINE_BLOCK, encoding="utf-8")

        raw_output = "\n".join(
            f"GAS_MEASUREMENT: {fn}: {sz}: {cost}"
            for fn, sizes in measurements.items()
            for sz, cost in (sizes.items() if isinstance(sizes, dict) else [("single", sizes)])
        )

        monkeypatch.setattr(
            vg.subprocess, "run",
            lambda *a, **kw: types.SimpleNamespace(stdout=raw_output),
        )
        # Override file path used by main() via extract_baselines
        monkeypatch.setattr(vg, "extract_baselines", lambda _: baseline_override or {
            "create_stream": 1000,
            "withdraw": 500,
            "batch_withdraw": {"small": 200, "large": 800},
        })

    def test_no_regression_exits_0(self, tmp_path, monkeypatch):
        self._patch(monkeypatch, tmp_path, {
            "create_stream": 1000,
            "withdraw": 500,
            "batch_withdraw": {"small": 200, "large": 800},
        })
        with pytest.raises(SystemExit) as exc:
            vg.main()
        assert exc.value.code == 0

    def test_regression_exits_1(self, tmp_path, monkeypatch):
        # create_stream goes 30% over baseline
        self._patch(monkeypatch, tmp_path, {
            "create_stream": 1300,
            "withdraw": 500,
        })
        with pytest.raises(SystemExit) as exc:
            vg.main()
        assert exc.value.code == 1

    def test_no_measurements_exits_1(self, tmp_path, monkeypatch):
        monkeypatch.setattr(
            vg.subprocess, "run",
            lambda *a, **kw: types.SimpleNamespace(stdout=""),
        )
        monkeypatch.setattr(vg, "extract_baselines", lambda _: {"create_stream": 1000})
        with pytest.raises(SystemExit) as exc:
            vg.main()
        assert exc.value.code == 1

    def test_missing_baseline_key_shows_missing(self, tmp_path, monkeypatch, capsys):
        # Measured function not in baseline -> printed as MISSING
        self._patch(monkeypatch, tmp_path, {"unknown_fn": 999},
                    baseline_override={"create_stream": 1000})
        with pytest.raises(SystemExit):
            vg.main()
        assert "MISSING" in capsys.readouterr().out

    def test_extract_baselines_exception_exits_1(self, tmp_path, monkeypatch):
        def _raise(_):
            raise ValueError("bad block")

        monkeypatch.setattr(vg, "extract_baselines", _raise)
        monkeypatch.setattr(
            vg.subprocess, "run",
            lambda *a, **kw: types.SimpleNamespace(stdout=""),
        )
        with pytest.raises(SystemExit) as exc:
            vg.main()
        assert exc.value.code == 1

    def test_pass_prints_success_message(self, tmp_path, monkeypatch, capsys):
        self._patch(monkeypatch, tmp_path, {
            "create_stream": 1000,
            "withdraw": 500,
        })
        with pytest.raises(SystemExit):
            vg.main()
        assert "SUCCESS" in capsys.readouterr().out

    def test_fail_prints_failed_message(self, tmp_path, monkeypatch, capsys):
        self._patch(monkeypatch, tmp_path, {"create_stream": 2000})
        with pytest.raises(SystemExit):
            vg.main()
        assert "FAILED" in capsys.readouterr().out

    def test_batch_withdraw_baseline_lookup(self, tmp_path, monkeypatch):
        # batch_withdraw sizes are looked up via baselines["batch_withdraw"][size]
        self._patch(monkeypatch, tmp_path, {
            "batch_withdraw": {"small": 200, "large": 800},
        })
        with pytest.raises(SystemExit) as exc:
            vg.main()
        assert exc.value.code == 0

    def test_within_5pct_passes(self, tmp_path, monkeypatch):
        # 4% increase is within tolerance
        self._patch(monkeypatch, tmp_path, {"create_stream": 1040})
        with pytest.raises(SystemExit) as exc:
            vg.main()
        assert exc.value.code == 0

    def test_exactly_5pct_passes(self, tmp_path, monkeypatch):
        # exactly 5% is not strictly > 0.05, so passes
        self._patch(monkeypatch, tmp_path, {"create_stream": 1050})
        with pytest.raises(SystemExit) as exc:
            vg.main()
        assert exc.value.code == 0

    def test_over_5pct_fails(self, tmp_path, monkeypatch):
        self._patch(monkeypatch, tmp_path, {"create_stream": 1060})
        with pytest.raises(SystemExit) as exc:
            vg.main()
        assert exc.value.code == 1


# ---------------------------------------------------------------------------
# Additional tests for check_duplicate_discriminants coverage
# ---------------------------------------------------------------------------

class TestCheckDuplicateDiscriminants:
    def test_no_contract_error_enum_returns_false(self, capsys):
        """Test that missing ContractError enum returns False with warning."""
        source = "pub enum SomeOtherEnum { A = 1, B = 2 }"
        result = vda.check_duplicate_discriminants(source)
        assert result is False
        assert "WARNING: could not locate 'pub enum ContractError'" in capsys.readouterr().out

    def test_duplicate_discriminants_detected(self, capsys):
        """Test that duplicate discriminants are detected and printed."""
        source = """\
pub enum ContractError {
    ErrorA = 1,
    ErrorB = 2,
    ErrorC = 1,
}
"""
        result = vda.check_duplicate_discriminants(source)
        assert result is True
        out = capsys.readouterr().out
        assert "DUPLICATE DISCRIMINANT" in out
        assert "ErrorA" in out
        assert "ErrorC" in out

    def test_no_duplicates_returns_false(self):
        """Test that no duplicates returns False."""
        source = """\
pub enum ContractError {
    ErrorA = 1,
    ErrorB = 2,
    ErrorC = 3,
}
"""
        result = vda.check_duplicate_discriminants(source)
        assert result is False

    def test_excluded_variants_ignored(self):
        """Test that excluded variants don't trigger duplicate detection."""
        source = """\
pub enum ContractError {
    Operational = 1,
    Administrative = 1,
    ErrorA = 2,
}
"""
        # Both Operational and Administrative use value 1, but they're in ERROR_EXTRACT_EXCLUDE
        result = vda.check_duplicate_discriminants(source)
        assert result is False


# ---------------------------------------------------------------------------
# Shared fixtures for audit.md drift tests
# ---------------------------------------------------------------------------

# Minimal lib.rs with a #[contractimpl] block containing two pub fns.
_CONTRACTIMPL_SRC = """\
pub fn save_stream(env: &Env) {}

#[contractimpl]
impl FluxoraStream {
    pub fn init(env: Env) -> Result<(), Error> { Ok(()) }
    pub fn withdraw(env: Env) -> Result<i128, Error> { Ok(0) }
    pub fn upgrade(env: Env) {}
}

pub fn compute_keeper_fee_split(gross: i128, bps: u32) -> (i128, i128) { (0, 0) }
"""

# audit.md snippet that documents both ABI entrypoints.
_AUDIT_DOC_FULL = """\
## Public entrypoints

| Entrypoint | Parameters | Return type | Authorization | Description |
| --- | --- | --- | --- | --- |
| `init` | `env: Env` | — | Bootstrap admin | One-time setup. |
| `withdraw` | `env: Env`, `stream_id: u64` | `i128` | Recipient | Withdraw accrued. |
"""

# audit.md missing the `withdraw` row.
_AUDIT_DOC_PARTIAL = """\
## Public entrypoints

| Entrypoint | Parameters | Return type | Authorization | Description |
| --- | --- | --- | --- | --- |
| `init` | `env: Env` | — | Bootstrap admin | One-time setup. |
"""

# audit.md with no table at all.
_AUDIT_DOC_EMPTY = "# Audit preparation\n\nNo table here.\n"


# ---------------------------------------------------------------------------
# extract_contractimpl_entrypoints
# ---------------------------------------------------------------------------

class TestExtractContractimplEntrypoints:
    """Tests for extract_contractimpl_entrypoints()."""

    def test_returns_set_type(self):
        result = vda.extract_contractimpl_entrypoints(_CONTRACTIMPL_SRC)
        assert isinstance(result, set)

    def test_finds_init_inside_contractimpl(self):
        result = vda.extract_contractimpl_entrypoints(_CONTRACTIMPL_SRC)
        assert "init" in result

    def test_finds_withdraw_inside_contractimpl(self):
        result = vda.extract_contractimpl_entrypoints(_CONTRACTIMPL_SRC)
        assert "withdraw" in result

    def test_excludes_save_stream_module_level(self):
        # save_stream is a module-level pub fn, not inside #[contractimpl].
        result = vda.extract_contractimpl_entrypoints(_CONTRACTIMPL_SRC)
        assert "save_stream" not in result

    def test_excludes_compute_keeper_fee_split_via_allowlist(self):
        # compute_keeper_fee_split appears after the impl block and is
        # in AUDIT_ENTRYPOINT_ALLOWLIST.
        result = vda.extract_contractimpl_entrypoints(_CONTRACTIMPL_SRC)
        assert "compute_keeper_fee_split" not in result

    def test_excludes_upgrade_via_allowlist(self):
        # upgrade is inside the contractimpl block but is in
        # AUDIT_ENTRYPOINT_ALLOWLIST, so it must be excluded.
        result = vda.extract_contractimpl_entrypoints(_CONTRACTIMPL_SRC)
        assert "upgrade" not in result

    def test_no_contractimpl_returns_empty_set(self):
        src = "pub fn orphan(env: Env) {}\n"
        assert vda.extract_contractimpl_entrypoints(src) == set()

    def test_empty_source_returns_empty_set(self):
        assert vda.extract_contractimpl_entrypoints("") == set()

    def test_multiple_entrypoints_all_found(self):
        src = (
            "#[contractimpl]\n"
            "impl Foo {\n"
            "    pub fn alpha(env: Env) {}\n"
            "    pub fn beta(env: Env) {}\n"
            "    pub fn gamma(env: Env) {}\n"
            "}\n"
        )
        result = vda.extract_contractimpl_entrypoints(src)
        assert {"alpha", "beta", "gamma"}.issubset(result)

    def test_private_fn_inside_contractimpl_not_returned(self):
        src = (
            "#[contractimpl]\n"
            "impl Foo {\n"
            "    pub fn public_fn(env: Env) {}\n"
            "    fn private_fn(env: Env) {}\n"
            "}\n"
        )
        result = vda.extract_contractimpl_entrypoints(src)
        assert "public_fn" in result
        assert "private_fn" not in result

    def test_nested_braces_do_not_confuse_scanner(self):
        src = (
            "#[contractimpl]\n"
            "impl Foo {\n"
            "    pub fn complex(env: Env) {\n"
            "        let x = { 1 + 2 };\n"
            "        if true { let _y = { 3 }; }\n"
            "    }\n"
            "    pub fn simple(env: Env) {}\n"
            "}\n"
        )
        result = vda.extract_contractimpl_entrypoints(src)
        assert "complex" in result
        assert "simple" in result

    def test_module_level_pub_fn_after_impl_not_included(self):
        src = (
            "#[contractimpl]\n"
            "impl Foo {\n"
            "    pub fn inside(env: Env) {}\n"
            "}\n"
            "pub fn outside(env: Env) {}\n"
        )
        result = vda.extract_contractimpl_entrypoints(src)
        assert "inside" in result
        assert "outside" not in result

    def test_allowlist_names_filtered_even_inside_contractimpl(self):
        src = (
            "#[contractimpl]\n"
            "impl Foo {\n"
            "    pub fn upgrade(env: Env) {}\n"
            "    pub fn compute_keeper_fee_split(x: i128) -> (i128, i128) { (0, 0) }\n"
            "    pub fn real_entrypoint(env: Env) {}\n"
            "}\n"
        )
        result = vda.extract_contractimpl_entrypoints(src)
        assert "real_entrypoint" in result
        assert "upgrade" not in result
        assert "compute_keeper_fee_split" not in result

    def test_generic_pub_fn_inside_contractimpl(self):
        src = (
            "#[contractimpl]\n"
            "impl Foo {\n"
            "    pub fn generic_fn<T>(x: T) {}\n"
            "}\n"
        )
        result = vda.extract_contractimpl_entrypoints(src)
        assert "generic_fn" in result


# ---------------------------------------------------------------------------
# extract_audit_entrypoints_from_doc
# ---------------------------------------------------------------------------

class TestExtractAuditEntrypointsFromDoc:
    """Tests for extract_audit_entrypoints_from_doc()."""

    def test_returns_set_type(self):
        result = vda.extract_audit_entrypoints_from_doc(_AUDIT_DOC_FULL)
        assert isinstance(result, set)

    def test_finds_init_in_full_doc(self):
        result = vda.extract_audit_entrypoints_from_doc(_AUDIT_DOC_FULL)
        assert "init" in result

    def test_finds_withdraw_in_full_doc(self):
        result = vda.extract_audit_entrypoints_from_doc(_AUDIT_DOC_FULL)
        assert "withdraw" in result

    def test_empty_doc_returns_empty_set(self):
        assert vda.extract_audit_entrypoints_from_doc("") == set()

    def test_no_table_returns_empty_set(self):
        assert vda.extract_audit_entrypoints_from_doc(_AUDIT_DOC_EMPTY) == set()

    def test_partial_doc_missing_withdraw(self):
        result = vda.extract_audit_entrypoints_from_doc(_AUDIT_DOC_PARTIAL)
        assert "init" in result
        assert "withdraw" not in result

    def test_deduplicates_repeated_rows(self):
        doc = (
            "| `init` | ... |\n"
            "| `init` | ... |\n"
            "| `withdraw` | ... |\n"
        )
        result = vda.extract_audit_entrypoints_from_doc(doc)
        assert result == {"init", "withdraw"}

    def test_ignores_header_row_dashes(self):
        # Table separator lines like | --- | should not produce identifiers.
        doc = (
            "| Entrypoint | Description |\n"
            "| --- | --- |\n"
            "| `init` | One-time setup. |\n"
        )
        result = vda.extract_audit_entrypoints_from_doc(doc)
        assert "---" not in result
        assert "init" in result

    def test_handles_indented_table_rows(self):
        doc = "   | `create_stream` | ... |\n"
        result = vda.extract_audit_entrypoints_from_doc(doc)
        assert "create_stream" in result

    def test_alphanumeric_underscored_names_extracted(self):
        doc = "| `get_stream_state_2` | ... |\n"
        result = vda.extract_audit_entrypoints_from_doc(doc)
        assert "get_stream_state_2" in result


# ---------------------------------------------------------------------------
# check_audit_md_entrypoint_drift
# ---------------------------------------------------------------------------

class TestCheckAuditMdEntrypointDrift:
    """Tests for check_audit_md_entrypoint_drift()."""

    def test_returns_false_when_all_documented(self, tmp_path):
        audit_path = tmp_path / "audit.md"
        audit_path.write_text(_AUDIT_DOC_FULL, encoding="utf-8")
        result = vda.check_audit_md_entrypoint_drift(
            _CONTRACTIMPL_SRC, _AUDIT_DOC_FULL, audit_path
        )
        assert result is False

    def test_returns_true_when_entrypoint_missing(self, tmp_path):
        audit_path = tmp_path / "audit.md"
        audit_path.write_text(_AUDIT_DOC_PARTIAL, encoding="utf-8")
        result = vda.check_audit_md_entrypoint_drift(
            _CONTRACTIMPL_SRC, _AUDIT_DOC_PARTIAL, audit_path
        )
        assert result is True

    def test_prints_missing_audit_doc_tag(self, tmp_path, capsys):
        audit_path = tmp_path / "audit.md"
        result = vda.check_audit_md_entrypoint_drift(
            _CONTRACTIMPL_SRC, _AUDIT_DOC_PARTIAL, audit_path
        )
        assert result is True
        assert "MISSING AUDIT DOC:" in capsys.readouterr().out

    def test_missing_message_contains_function_name(self, tmp_path, capsys):
        audit_path = tmp_path / "audit.md"
        vda.check_audit_md_entrypoint_drift(
            _CONTRACTIMPL_SRC, _AUDIT_DOC_PARTIAL, audit_path
        )
        assert "withdraw" in capsys.readouterr().out

    def test_missing_message_contains_doc_filename(self, tmp_path, capsys):
        audit_path = tmp_path / "audit.md"
        # Patch REPO_ROOT so relative_to() works
        orig = vda.REPO_ROOT
        vda.REPO_ROOT = tmp_path
        try:
            vda.check_audit_md_entrypoint_drift(
                _CONTRACTIMPL_SRC, _AUDIT_DOC_PARTIAL, audit_path
            )
        finally:
            vda.REPO_ROOT = orig
        assert "audit.md" in capsys.readouterr().out

    def test_no_contractimpl_block_no_drift(self, tmp_path):
        # If there's no #[contractimpl] block, no entrypoints are expected.
        src = "pub fn orphan(env: Env) {}\n"
        audit_path = tmp_path / "audit.md"
        result = vda.check_audit_md_entrypoint_drift(
            src, _AUDIT_DOC_EMPTY, audit_path
        )
        assert result is False

    def test_empty_source_no_drift(self, tmp_path):
        audit_path = tmp_path / "audit.md"
        result = vda.check_audit_md_entrypoint_drift(
            "", _AUDIT_DOC_EMPTY, audit_path
        )
        assert result is False

    def test_allowlisted_entrypoints_not_flagged(self, tmp_path):
        # upgrade and compute_keeper_fee_split inside contractimpl should
        # not be flagged even when absent from audit.md.
        src = (
            "#[contractimpl]\n"
            "impl Foo {\n"
            "    pub fn upgrade(env: Env) {}\n"
            "    pub fn compute_keeper_fee_split(x: i128) -> (i128, i128) { (0, 0) }\n"
            "}\n"
        )
        audit_path = tmp_path / "audit.md"
        result = vda.check_audit_md_entrypoint_drift(
            src, _AUDIT_DOC_EMPTY, audit_path
        )
        assert result is False

    def test_multiple_missing_all_printed(self, tmp_path, capsys):
        src = (
            "#[contractimpl]\n"
            "impl Foo {\n"
            "    pub fn alpha(env: Env) {}\n"
            "    pub fn beta(env: Env) {}\n"
            "    pub fn gamma(env: Env) {}\n"
            "}\n"
        )
        audit_path = tmp_path / "audit.md"
        vda.check_audit_md_entrypoint_drift(src, _AUDIT_DOC_EMPTY, audit_path)
        out = capsys.readouterr().out
        assert "alpha" in out
        assert "beta" in out
        assert "gamma" in out

    def test_path_outside_repo_root_falls_back_to_full_path(self, tmp_path, capsys):
        # If audit_doc_path is outside REPO_ROOT, relative_to raises ValueError;
        # the function should gracefully fall back to the absolute path.
        outside_path = tmp_path / "elsewhere" / "audit.md"
        outside_path.parent.mkdir(parents=True, exist_ok=True)
        # REPO_ROOT does NOT contain outside_path
        orig = vda.REPO_ROOT
        vda.REPO_ROOT = tmp_path / "repo"
        try:
            vda.check_audit_md_entrypoint_drift(
                _CONTRACTIMPL_SRC, _AUDIT_DOC_PARTIAL, outside_path
            )
        finally:
            vda.REPO_ROOT = orig
        out = capsys.readouterr().out
        assert "MISSING AUDIT DOC:" in out

    def test_outputs_are_sorted_alphabetically(self, tmp_path, capsys):
        src = (
            "#[contractimpl]\n"
            "impl Foo {\n"
            "    pub fn zzz_last(env: Env) {}\n"
            "    pub fn aaa_first(env: Env) {}\n"
            "    pub fn mmm_mid(env: Env) {}\n"
            "}\n"
        )
        audit_path = tmp_path / "audit.md"
        vda.check_audit_md_entrypoint_drift(src, _AUDIT_DOC_EMPTY, audit_path)
        out = capsys.readouterr().out
        lines = [l for l in out.splitlines() if "MISSING AUDIT DOC:" in l]
        names = [l.split("'")[1] for l in lines]
        assert names == sorted(names)

    def test_stale_doc_rows_not_flagged(self, tmp_path):
        # audit.md may document names that no longer exist in code;
        # that is NOT flagged by this check (additive-only).
        src = (
            "#[contractimpl]\n"
            "impl Foo {\n"
            "    pub fn init(env: Env) {}\n"
            "}\n"
        )
        doc = _AUDIT_DOC_FULL  # documents `init` AND `withdraw`; withdraw gone from code
        audit_path = tmp_path / "audit.md"
        result = vda.check_audit_md_entrypoint_drift(src, doc, audit_path)
        assert result is False


# ---------------------------------------------------------------------------
# validate() with audit_doc parameter
# ---------------------------------------------------------------------------

def _make_audit_files(tmp_path, audit_content=None):
    """Write all files including audit.md and return paths tuple.

    Uses a combined lib.rs that:
    - Satisfies the streaming.md entrypoint check (init, create_stream, withdraw
      plus upgrade and compute_keeper_fee_split are documented in the streaming stub)
    - Contains a #[contractimpl] block with init/withdraw for the audit check
    - Has upgrade and compute_keeper_fee_split as module-level pub fns that are
      in AUDIT_ENTRYPOINT_ALLOWLIST and also mentioned in the streaming stub
    """
    # Streaming doc that documents all pub fn names that appear in _CONTRACTIMPL_SRC
    # plus MINIMAL_LIB_RS (init, create_stream, withdraw, upgrade,
    # compute_keeper_fee_split). save_stream is in ENTRYPOINT_ALLOWLIST.
    streaming_doc = (
        "# Streaming\n"
        "`init`, `create_stream`, `withdraw`, `upgrade`, `compute_keeper_fee_split`\n"
    )
    lib_rs, events_rs, error_rs, streaming, events, error = _write_files(
        tmp_path, streaming=streaming_doc
    )
    if audit_content is None:
        audit_content = _AUDIT_DOC_FULL
    # Rewrite lib.rs to contain the contractimpl block with init/withdraw
    # plus module-level helpers upgrade and compute_keeper_fee_split.
    lib_rs.write_text(_CONTRACTIMPL_SRC + MINIMAL_LIB_RS, encoding="utf-8")
    audit_path = tmp_path / "audit.md"
    audit_path.write_text(audit_content, encoding="utf-8")
    return lib_rs, events_rs, error_rs, streaming, events, error, audit_path


class TestValidateWithAuditDoc:
    """Tests for validate() when audit_doc is supplied."""

    def test_passes_with_full_audit_doc(self, tmp_path):
        paths = _make_audit_files(tmp_path, _AUDIT_DOC_FULL)
        result = vda.validate(*paths[:6], audit_doc=paths[6])
        assert result == 0

    def test_fails_with_partial_audit_doc(self, tmp_path):
        paths = _make_audit_files(tmp_path, _AUDIT_DOC_PARTIAL)
        result = vda.validate(*paths[:6], audit_doc=paths[6])
        assert result == 1

    def test_audit_doc_none_skips_check(self, tmp_path):
        # When audit_doc=None, audit.md check is entirely skipped.
        paths = _make_audit_files(tmp_path, _AUDIT_DOC_EMPTY)
        # Without audit_doc, only streaming/events/error checks run.
        result = vda.validate(*paths[:6], audit_doc=None)
        # streaming/events/error should still pass with default MINIMAL stubs.
        assert result == 0

    def test_prints_missing_audit_doc_on_drift(self, tmp_path, capsys):
        paths = _make_audit_files(tmp_path, _AUDIT_DOC_PARTIAL)
        vda.validate(*paths[:6], audit_doc=paths[6])
        assert "MISSING AUDIT DOC:" in capsys.readouterr().out

    def test_ok_message_printed_when_audit_passes(self, tmp_path, capsys):
        paths = _make_audit_files(tmp_path, _AUDIT_DOC_FULL)
        vda.validate(*paths[:6], audit_doc=paths[6])
        assert "OK:" in capsys.readouterr().out

    def test_ok_message_not_printed_when_audit_fails(self, tmp_path, capsys):
        paths = _make_audit_files(tmp_path, _AUDIT_DOC_PARTIAL)
        vda.validate(*paths[:6], audit_doc=paths[6])
        assert "OK:" not in capsys.readouterr().out


# ---------------------------------------------------------------------------
# main() with DOC_AUDIT in MAPPING
# ---------------------------------------------------------------------------

def _fake_mapping_with_audit(tmp_path, files_7, missing_key=None):
    """Build a MAPPING dict that includes DOC_AUDIT."""
    keys = [
        "CONTRACT_SRC", "EVENTS_SRC", "ERROR_SRC",
        "DOC_STREAMING", "DOC_EVENTS", "DOC_ERROR", "DOC_AUDIT",
    ]
    names = [
        "lib.rs", "events.rs", "error.rs",
        "streaming.md", "events.md", "error.md", "audit.md",
    ]
    mapping = {}
    for key, name, path in zip(keys, names, files_7):
        if key == missing_key:
            mapping[key] = (tmp_path / "no_such_audit_xyz.md", "**/no_such_audit_xyz.md")
        else:
            mapping[key] = (path, f"**/{name}")
    return mapping


class TestMainWithAuditDoc:
    """Tests for main() exercising the DOC_AUDIT mapping entry."""

    def _setup(self, tmp_path, monkeypatch, audit_content=None):
        paths = _make_audit_files(tmp_path, audit_content)
        mapping = _fake_mapping_with_audit(tmp_path, paths)
        monkeypatch.setattr(vda, "MAPPING", mapping)
        monkeypatch.setattr(vda, "REPO_ROOT", tmp_path)
        return paths

    def test_main_passes_with_full_audit_doc(self, tmp_path, monkeypatch):
        self._setup(tmp_path, monkeypatch, _AUDIT_DOC_FULL)
        assert vda.main() == 0

    def test_main_fails_with_partial_audit_doc(self, tmp_path, monkeypatch):
        self._setup(tmp_path, monkeypatch, _AUDIT_DOC_PARTIAL)
        assert vda.main() == 1

    def test_main_missing_audit_doc_file_returns_1(self, tmp_path, monkeypatch):
        paths = _make_audit_files(tmp_path, _AUDIT_DOC_FULL)
        mapping = _fake_mapping_with_audit(tmp_path, paths, missing_key="DOC_AUDIT")
        monkeypatch.setattr(vda, "MAPPING", mapping)
        monkeypatch.setattr(vda, "REPO_ROOT", tmp_path)
        assert vda.main() == 1

    def test_main_prints_missing_audit_doc_tag_on_drift(
            self, tmp_path, monkeypatch, capsys):
        self._setup(tmp_path, monkeypatch, _AUDIT_DOC_PARTIAL)
        vda.main()
        assert "MISSING AUDIT DOC:" in capsys.readouterr().out

    def test_main_ok_when_all_entrypoints_documented(
            self, tmp_path, monkeypatch, capsys):
        self._setup(tmp_path, monkeypatch, _AUDIT_DOC_FULL)
        vda.main()
        assert "OK:" in capsys.readouterr().out

    def test_audit_allowlist_members_do_not_cause_failure(
            self, tmp_path, monkeypatch):
        # upgrade and compute_keeper_fee_split are in AUDIT_ENTRYPOINT_ALLOWLIST;
        # even if absent from audit.md, main() must still return 0.
        src_with_allowlisted = (
            _CONTRACTIMPL_SRC  # contains upgrade inside contractimpl
            + MINIMAL_LIB_RS
        )
        paths = _make_audit_files(tmp_path, _AUDIT_DOC_FULL)
        paths[0].write_text(src_with_allowlisted, encoding="utf-8")
        mapping = _fake_mapping_with_audit(tmp_path, paths)
        monkeypatch.setattr(vda, "MAPPING", mapping)
        monkeypatch.setattr(vda, "REPO_ROOT", tmp_path)
        assert vda.main() == 0
