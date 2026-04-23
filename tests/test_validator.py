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

    def test_does_not_match_old_macro_style(self):
        assert "old_style" not in vda.extract_event_symbols(
            'symbol_short!("old_style")')

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
