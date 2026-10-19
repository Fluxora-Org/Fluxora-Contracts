"""The stream size gate must preserve the other verified release artifacts."""
import os
from pathlib import Path
import subprocess


REPO = Path(__file__).parents[1]


def run_size_gate(tmp_path, size):
    release = tmp_path / "target/wasm32v1-none/release"
    release.mkdir(parents=True)
    preserved = {
        "fluxora_factory.wasm": b"factory artifact",
        "fluxora_stream.optimized.wasm": b"optimized artifact",
        "fluxora_stream.wasm.sha256": b"verified checksum",
    }
    for name, contents in preserved.items():
        (release / name).write_bytes(contents)
    binary_dir = tmp_path / "bin"
    binary_dir.mkdir()
    cargo = binary_dir / "cargo"
    cargo.write_text(
        "#!/usr/bin/env python3\n"
        "import os, pathlib, shutil, sys\n"
        "target = pathlib.Path('target/wasm32v1-none')\n"
        "if sys.argv[1] == 'clean':\n"
        "    shutil.rmtree(target, ignore_errors=True)\n"
        "elif sys.argv[1] == 'build':\n"
        "    release = target / 'release'\n"
        "    release.mkdir(parents=True, exist_ok=True)\n"
        "    (release / 'fluxora_stream.wasm').write_bytes(b'x' * int(os.environ['FAKE_WASM_BYTES']))\n"
        "else:\n"
        "    sys.exit(2)\n",
        encoding="utf-8",
    )
    cargo.chmod(0o755)
    env = dict(os.environ, FAKE_WASM_BYTES=str(size))
    env["PATH"] = str(binary_dir) + os.pathsep + env["PATH"]
    result = subprocess.run(
        ["bash", str(REPO / "script/check-stream-wasm-size.sh"),
         str(REPO / "contracts/stream/wasm-size-budget.env")],
        cwd=tmp_path, env=env, capture_output=True, text=True,
    )
    return result, release, preserved


def test_size_gate_preserves_other_verified_release_artifacts(tmp_path):
    result, release, preserved = run_size_gate(tmp_path, 100)
    assert result.returncode == 0, result.stderr
    for name, contents in preserved.items():
        assert (release / name).read_bytes() == contents


def test_size_gate_still_rejects_oversized_wasm(tmp_path):
    result, _, _ = run_size_gate(tmp_path, 1_000_000)
    assert result.returncode != 0
    assert "exceeds budget" in result.stderr
