import json
import os
import re
import subprocess
import sys
from pathlib import Path
from typing import Any, Dict

BULK_BATCH_BASELINE_SIZES = ("1", "10", "50", "100")
REQUIRED_BULK_BASELINES = {
    "bulk_cancel_streams": BULK_BATCH_BASELINE_SIZES,
    "bulk_resume_streams_as_admin": BULK_BATCH_BASELINE_SIZES,
}

RELEASE_HARDENING_START = "<!-- RELEASE_HARDENING_COVERAGE_START -->"
RELEASE_HARDENING_END = "<!-- RELEASE_HARDENING_COVERAGE_END -->"
REQUIRED_RELEASE_HARDENING_REFERENCES = {
    "Storage": (
        "contracts/stream/tests/storage_invariants_edge_cases.rs",
        "contracts/stream/tests/storage_key_compat.rs",
        "contracts/stream/tests/security_invariants.rs",
    ),
    "Gas": (
        "contracts/stream/tests/gas_regression.rs",
        "tests/test_gas_validation.py",
        "script/check-wasm-size.sh",
    ),
    "Upgrade": (
        "contracts/stream/tests/upgrade_path.rs",
        "contracts/stream/tests/storage_key_compat.rs",
    ),
    "Compatibility": (
        "contracts/stream/tests/storage_key_compat.rs",
        "contracts/stream/tests/security_invariants.rs",
        "contracts/stream/tests/event_snapshots_suite.rs",
    ),
}


def build_cargo_test_env() -> Dict[str, str]:
    """Return the inherited environment with ~/.cargo/bin prepended to PATH."""
    home = os.environ.get("HOME")
    if not home:
        raise RuntimeError(
            "HOME is not set; cannot prepend ~/.cargo/bin to PATH for cargo. "
            "Set HOME in the environment or ensure cargo is on PATH."
        )
    env = os.environ.copy()
    env["PATH"] = f"{home}/.cargo/bin:{env.get('PATH', '')}"
    return env


def extract_baselines(file_path: str) -> Dict[str, Any]:
    with open(file_path, "r", encoding="utf-8") as file:
        content = file.read()
    start_marker = "<!-- GAS_BASELINE_START -->"
    end_marker = "<!-- GAS_BASELINE_END -->"
    start = content.find(start_marker)
    end = content.find(end_marker)
    if start == -1 or end == -1 or end <= start:
        raise ValueError("Could not find gas baseline block in docs/gas.md")
    block = content[start + len(start_marker) : end].strip()
    if not block.startswith("{"):
        raise ValueError("Could not find gas baseline block in docs/gas.md")
    baselines = json.loads(block)
    if not isinstance(baselines, dict):
        raise ValueError("Gas baseline block must contain a JSON object")
    return baselines


def validate_required_baselines(baselines: Dict[str, Any]) -> None:
    """Ensure every legacy bulk-batch size has a documented baseline."""
    missing = []
    for function, sizes in REQUIRED_BULK_BASELINES.items():
        raw = baselines.get(function)
        if not isinstance(raw, dict):
            missing.append(f"{function}: all")
            continue
        for size in sizes:
            if size not in raw:
                missing.append(f"{function}: {size}")

    if missing:
        raise ValueError(f"Missing required gas baselines: {', '.join(missing)}")


def extract_release_hardening_coverage(file_path: str) -> str:
    """Extract the machine-guarded release-hardening coverage matrix."""
    content = Path(file_path).read_text(encoding="utf-8")
    start = content.find(RELEASE_HARDENING_START)
    end = content.find(RELEASE_HARDENING_END)
    if start == -1 or end == -1 or end <= start:
        raise ValueError("Could not find release-hardening coverage block in docs/gas.md")
    return content[start + len(RELEASE_HARDENING_START) : end].strip()


def validate_release_hardening_coverage(
    file_path: str, repo_root: str | Path | None = None
) -> None:
    """Guard the storage/gas/upgrade/compatibility coverage map against drift."""
    section = extract_release_hardening_coverage(file_path)
    root = (
        Path(repo_root)
        if repo_root is not None
        else Path(file_path).resolve().parent.parent
    )

    missing_dimensions = []
    missing_references = []
    missing_files = []
    for dimension, references in REQUIRED_RELEASE_HARDENING_REFERENCES.items():
        if f"**{dimension}**" not in section:
            missing_dimensions.append(dimension)
        for reference in references:
            if f"`{reference}`" not in section:
                missing_references.append(f"{dimension}: {reference}")
            if not (root / reference).is_file():
                missing_files.append(reference)

    errors = []
    if missing_dimensions:
        errors.append("missing dimensions: " + ", ".join(missing_dimensions))
    if missing_references:
        errors.append("missing test references: " + ", ".join(missing_references))
    if missing_files:
        errors.append("referenced files do not exist: " + ", ".join(sorted(set(missing_files))))
    if errors:
        raise ValueError("Incomplete release-hardening coverage: " + "; ".join(errors))


def run_tests() -> str:
    """Run the metered Rust suite and return stdout, failing on build/test errors."""
    print("Running gas regression tests...")
    result = subprocess.run(
        [
            "cargo",
            "test",
            "-p",
            "fluxora_stream",
            "--test",
            "gas_regression",
            "--",
            "--nocapture",
        ],
        capture_output=True,
        text=True,
        env=build_cargo_test_env(),
    )
    if result.returncode != 0:
        details = (result.stderr or result.stdout).strip()
        if len(details) > 4000:
            details = details[-4000:]
        raise RuntimeError(
            f"gas regression test command failed with exit code {result.returncode}"
            + (f":\n{details}" if details else "")
        )
    return result.stdout


def parse_measurements(output: str) -> Dict[str, Dict[str, int]]:
    measurements: Dict[str, Dict[str, int]] = {}
    pattern = re.compile(r"GAS_MEASUREMENT: ([^:]+): ([^:]+): (\d+)")
    for line in output.splitlines():
        match = pattern.search(line)
        if match:
            function, size, cost = match.groups()
            measurements.setdefault(function, {})[size] = int(cost)
    return measurements


def main() -> None:
    try:
        gas_doc = "docs/gas.md"
        baselines = extract_baselines(gas_doc)
        validate_release_hardening_coverage(gas_doc)
        output = run_tests()
        measured = parse_measurements(output)

        if not measured:
            raise ValueError("No gas measurements found in test output")

        regressions = []
        missing_baselines = []
        invalid_baselines = []
        print("\nGas Cost Report:")
        print(
            f"{'Function':<38} | {'Size':<10} | {'Baseline':<12} | "
            f"{'Measured':<12} | {'Diff %':<10} | Status"
        )
        print("-" * 110)

        for function, sizes in measured.items():
            for size, cost in sizes.items():
                raw = baselines.get(function)
                baseline = raw.get(size) if isinstance(raw, dict) else raw

                if baseline is None:
                    missing_baselines.append((function, size))
                    print(
                        f"{function:<38} | {size:<10} | {'N/A':<12} | "
                        f"{cost:<12} | {'N/A':<10} | MISSING"
                    )
                    continue
                if not isinstance(baseline, int) or isinstance(baseline, bool) or baseline <= 0:
                    invalid_baselines.append((function, size, baseline))
                    print(
                        f"{function:<38} | {size:<10} | {str(baseline):<12} | "
                        f"{cost:<12} | {'N/A':<10} | INVALID"
                    )
                    continue

                diff = (cost - baseline) / baseline
                # Preserve the documented strict boundary without float rounding:
                # exactly +5% passes; one instruction over the boundary fails.
                failed = cost * 100 > baseline * 105
                status = "FAIL" if failed else "PASS"
                if failed:
                    regressions.append((function, size, diff))
                print(
                    f"{function:<38} | {size:<10} | {baseline:<12} | "
                    f"{cost:<12} | {diff:>8.2%} | {status}"
                )

        if missing_baselines:
            print("\nFAILED: Measurements without a documented gas baseline:")
            for function, size in missing_baselines:
                print(f"- {function} ({size})")
        if invalid_baselines:
            print("\nFAILED: Gas baselines must be positive integers:")
            for function, size, baseline in invalid_baselines:
                print(f"- {function} ({size}): {baseline!r}")
        if regressions:
            print("\nFAILED: Gas regression detected (> 5% increase):")
            for function, size, diff in regressions:
                print(f"- {function} ({size}): {diff:.2%}")

        if missing_baselines or invalid_baselines or regressions:
            return sys.exit(1)

        print("\nSUCCESS: Every measurement has a valid baseline and no gas regression was detected.")
        return sys.exit(0)
    except Exception as error:
        print(f"Error during validation: {error}")
        return sys.exit(1)


if __name__ == "__main__":
    main()
