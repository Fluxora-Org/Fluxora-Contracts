#!/usr/bin/env python3
"""Guard deployable package & artifact names against accidental change.

Deployment tooling consumes two exact identifiers for the shipping contract:

  * the Cargo *package* name   ``fluxora-stream``
  * the compiled *artifact*    ``fluxora_stream.wasm``

A rename of either breaks that tooling silently (the backend/frontend fetch by
name, and the CI wasm-upload step pins the path). This script makes such a
change a hard CI failure instead of a silent production surprise.

It works in two layers:

1. A clean-checkout check (no build required): it reads ``cargo metadata`` and
   asserts every name declared in ``packaging.json`` is present, unchanged, and
   self-consistent with the derived ``.wasm`` filename.
2. An artifact check (opt-in via ``--require-artifact``): it additionally
   asserts the compiled ``.wasm`` exists at the path deployment expects. CI runs
   this after the contract build.

The canonical names live in ``packaging.json`` at the repository root. The only
supported way to rename is documented in MIGRATION.md under "Renaming a
deployable package".

Run the built-in regression tests with ``--selftest``.
"""

import argparse
import json
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DEFAULT_WASM_TARGET = "wasm32v1-none"


def load_expected(root: Path) -> list:
    path = root / "packaging.json"
    with open(path) as fh:
        data = json.load(fh)
    deployables = data.get("deployables", [])
    if not deployables:
        raise SystemExit(f"packaging.json declares no deployables: {path}")
    return deployables


def wasm_artifact_name(package: str) -> str:
    """The compiled contract filename cargo emits for a package.

    Cargo replaces dashes with underscores and appends ``.wasm`` for a
    ``cdylib`` built for a wasm target, so ``fluxora-stream`` ->
    ``fluxora_stream.wasm``.
    """
    return package.replace("-", "_") + ".wasm"


def run_cargo_metadata(manifest_path: Path, attempts: int = 3) -> dict:
    # cargo metadata resolves the dependency graph, which in CI can hit a
    # flaky registry mirror. Retry the invocation (never the assertions) so a
    # transient network error does not fail the build.
    last_err = ""
    for attempt in range(1, attempts + 1):
        proc = subprocess.run(
            [
                "cargo", "metadata", "--format-version", "1", "--no-deps",
                "--manifest-path", str(manifest_path),
            ],
            capture_output=True, text=True,
        )
        if proc.returncode == 0:
            return json.loads(proc.stdout)
        last_err = proc.stderr
        if attempt < attempts:
            time.sleep(2 * attempt)
    raise SystemExit(f"cargo metadata failed after {attempts} attempts:\n{last_err}")


def workspace_package_names(meta: dict) -> list:
    members = set(meta.get("workspace_members", []))
    names = []
    for pkg in meta.get("packages", []):
        # With --no-deps only workspace members are present, but keep the
        # membership filter so the function is correct without that flag too.
        if not members or pkg.get("id") in members:
            names.append(pkg["name"])
    return names


def verify(expected, meta, require_artifact: bool, wasm_target: str) -> list:
    errors = []
    names = workspace_package_names(meta)
    target_dir = Path(meta.get("target_directory", ""))

    for entry in expected:
        pkg = entry["package"]
        declared_wasm = entry["wasm"]

        if pkg not in names:
            errors.append(
                f"deployable package '{pkg}' not found in the workspace; "
                f"current workspace packages are {sorted(names)}. A deployable "
                f"was renamed or removed — follow the rename procedure in "
                f"MIGRATION.md."
            )

        derived = wasm_artifact_name(pkg)
        if derived != declared_wasm:
            errors.append(
                f"packaging.json is inconsistent: package '{pkg}' implies wasm "
                f"'{derived}' but declares '{declared_wasm}'. The two must "
                f"agree."
            )

        if require_artifact:
            artifact = target_dir / wasm_target / "release" / declared_wasm
            if not artifact.exists():
                errors.append(
                    f"deployable artifact missing at '{artifact}'. The compiled "
                    f"contract filename changed, so deployment tooling will not "
                    f"find it. Follow the rename procedure in MIGRATION.md."
                )

    return errors


def run_selftest() -> int:
    suite = unittest.TestLoader().loadTestsFromName(
        "GuardTests", sys.modules[__name__]
    )
    result = unittest.TextTestRunner(verbosity=2).run(suite)
    return 0 if result.wasSuccessful() else 1


class GuardTests(unittest.TestCase):
    def _meta(self, packages, members=None, target="/tmp/target"):
        if members is None:
            members = [p["id"] for p in packages]
        return {
            "packages": packages,
            "workspace_members": members,
            "target_directory": target,
        }

    def _pkg(self, name, pid=None):
        return {"id": pid or f"path+{name}", "name": name}

    def test_wasm_artifact_name(self):
        self.assertEqual(wasm_artifact_name("fluxora-stream"),
                         "fluxora_stream.wasm")
        self.assertEqual(wasm_artifact_name("a-b-c"), "a_b_c.wasm")
        self.assertEqual(wasm_artifact_name("solo"), "solo.wasm")

    def test_workspace_package_names_filters_non_members(self):
        meta = self._meta(
            [self._pkg("fluxora-stream", "m1"),
             self._pkg("dep-crate", "ext1")],
            members=["m1"],
        )
        self.assertEqual(workspace_package_names(meta), ["fluxora-stream"])

    def test_verify_happy_path(self):
        meta = self._meta([
            self._pkg("fluxora-stream"),
            self._pkg("fluxora-archival-probe"),
        ])
        expected = [{"package": "fluxora-stream", "wasm": "fluxora_stream.wasm"}]
        self.assertEqual(verify(expected, meta, False, "wasm32v1-none"), [])

    def test_verify_missing_package_is_failure(self):
        # fluxora-stream was renamed to fluxora-stream-v2.
        meta = self._meta([self._pkg("fluxora-stream-v2")])
        expected = [{"package": "fluxora-stream", "wasm": "fluxora_stream.wasm"}]
        errors = verify(expected, meta, False, "wasm32v1-none")
        self.assertEqual(len(errors), 1)
        self.assertIn("not found", errors[0])

    def test_verify_inconsistent_packaging_is_boundary_failure(self):
        meta = self._meta([self._pkg("fluxora-stream")])
        # declared wasm disagrees with what the package name implies.
        expected = [{"package": "fluxora-stream", "wasm": "fluxora_stream_v2.wasm"}]
        errors = verify(expected, meta, False, "wasm32v1-none")
        self.assertEqual(len(errors), 1)
        self.assertIn("inconsistent", errors[0])

    def test_verify_artifact_absent_without_flag_is_ok(self):
        meta = self._meta([self._pkg("fluxora-stream")])
        expected = [{"package": "fluxora-stream", "wasm": "fluxora_stream.wasm"}]
        errors = verify(expected, meta, False, "wasm32v1-none")
        self.assertEqual(errors, [])

    def test_verify_artifact_absent_with_flag_is_failure(self):
        meta = self._meta([self._pkg("fluxora-stream")], target="/no/such/target")
        expected = [{"package": "fluxora-stream", "wasm": "fluxora_stream.wasm"}]
        errors = verify(expected, meta, True, "wasm32v1-none")
        self.assertEqual(len(errors), 1)
        self.assertIn("missing", errors[0])

    def test_verify_artifact_present_with_flag_is_ok(self):
        with tempfile.TemporaryDirectory() as d:
            artifact = Path(d) / "wasm32v1-none" / "release" / "fluxora_stream.wasm"
            artifact.parent.mkdir(parents=True)
            artifact.write_text("")
            meta = self._meta([self._pkg("fluxora-stream")], target=d)
            expected = [{"package": "fluxora-stream", "wasm": "fluxora_stream.wasm"}]
            self.assertEqual(
                verify(expected, meta, True, "wasm32v1-none"), []
            )


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(
        description="Guard deployable package/artifact names."
    )
    parser.add_argument("--workspace-root", type=Path, default=ROOT)
    parser.add_argument(
        "--require-artifact", action="store_true",
        help="fail if the compiled .wasm artifact is absent",
    )
    parser.add_argument("--wasm-target", default=DEFAULT_WASM_TARGET,
                        help="wasm target triple the artifact is built for")
    parser.add_argument(
        "--selftest", action="store_true",
        help="run the built-in regression tests and exit",
    )
    args = parser.parse_args(argv)

    if args.selftest:
        return run_selftest()

    expected = load_expected(args.workspace_root)
    meta = run_cargo_metadata(args.workspace_root / "Cargo.toml")
    errors = verify(expected, meta, args.require_artifact, args.wasm_target)

    print("Deployable name guard")
    print(f"  workspace root : {args.workspace_root}")
    for entry in expected:
        print(f"  expected       : package={entry['package']} "
              f"wasm={entry['wasm']}")

    if errors:
        print("FAILED:")
        for err in errors:
            print(f"  - {err}")
        return 1

    print("OK: deployable package and artifact names match packaging.json")
    return 0


if __name__ == "__main__":
    sys.exit(main())
