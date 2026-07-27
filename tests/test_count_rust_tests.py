import importlib.util
from pathlib import Path


SCRIPT = Path(__file__).resolve().parents[1] / "script" / "count_rust_tests.py"


def _load_module():
    spec = importlib.util.spec_from_file_location("count_rust_tests", SCRIPT)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


count_rust_tests = _load_module()


def _write(path: Path, text: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text, encoding="utf-8")


def test_contract_rust_files_finds_src_and_tests(monkeypatch, tmp_path):
    root = tmp_path
    _write(root / "contracts" / "stream" / "src" / "lib.rs", "#[test]\nfn a() {}\n")
    _write(root / "contracts" / "stream" / "tests" / "flow.rs", "#[test]\nfn b() {}\n")
    _write(root / "contracts" / "stream" / "benches" / "ignored.rs", "#[test]\nfn c() {}\n")

    monkeypatch.setattr(count_rust_tests, "REPO_ROOT", root)

    found = {path.relative_to(root).as_posix() for path in count_rust_tests.contract_rust_files()}

    assert found == {
        "contracts/stream/src/lib.rs",
        "contracts/stream/tests/flow.rs",
    }


def test_count_tests_groups_by_crate(monkeypatch, tmp_path):
    root = tmp_path
    _write(
        root / "contracts" / "stream" / "src" / "lib.rs",
        "#[test]\nfn a() {}\n#[test]\nfn b() {}\n",
    )
    _write(root / "contracts" / "factory" / "tests" / "factory.rs", "#[test]\nfn c() {}\n")
    _write(root / "contracts" / "governance" / "src" / "lib.rs", "fn helper() {}\n")

    monkeypatch.setattr(count_rust_tests, "REPO_ROOT", root)

    total, by_crate = count_rust_tests.count_tests()

    assert total == 3
    assert by_crate == {"factory": 1, "stream": 2}


def test_main_prints_total_and_sorted_crates(monkeypatch, tmp_path, capsys):
    root = tmp_path
    _write(root / "contracts" / "stream" / "src" / "lib.rs", "#[test]\nfn a() {}\n")
    _write(root / "contracts" / "factory" / "src" / "lib.rs", "#[test]\nfn b() {}\n")

    monkeypatch.setattr(count_rust_tests, "REPO_ROOT", root)

    assert count_rust_tests.main() == 0
    assert capsys.readouterr().out.splitlines() == [
        "Total #[test] attributes: 2",
        "  factory: 1",
        "  stream: 1",
    ]
