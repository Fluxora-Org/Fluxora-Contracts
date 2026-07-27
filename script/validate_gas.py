import json
import os
import re
import subprocess
import sys
from typing import Any, Dict

BULK_BATCH_BASELINE_SIZES = ("1", "10", "50", "100")
REQUIRED_BULK_BASELINES = {
    "bulk_cancel_streams": BULK_BATCH_BASELINE_SIZES,
    "bulk_resume_streams_as_admin": BULK_BATCH_BASELINE_SIZES,
}


def build_cargo_test_env() -> Dict[str, str]:
    """Return subprocess env with ~/.cargo/bin prepended to PATH."""
    home = os.environ.get("HOME")
    if not home:
        raise RuntimeError(
            "HOME is not set; cannot prepend ~/.cargo/bin to PATH for cargo. "
            "Set HOME in the environment or ensure cargo is on PATH."
        )
    inherited_path = os.environ.get("PATH", "")
    return {"PATH": f"{home}/.cargo/bin:{inherited_path}"}

def extract_baselines(file_path: str) -> Dict[str, Any]:
    with open(file_path, 'r') as f:
        content = f.read()
        start_marker = '<!-- GAS_BASELINE_START -->'
        end_marker = '<!-- GAS_BASELINE_END -->'
        start = content.find(start_marker)
        end = content.find(end_marker)
        if start == -1 or end == -1 or end <= start:
            raise ValueError("Could not find gas baseline block in docs/gas.md")
        block = content[start + len(start_marker):end].strip()
        if not block.startswith('{'):
            raise ValueError("Could not find gas baseline block in docs/gas.md")
        return json.loads(block)

def validate_required_baselines(baselines: Dict[str, Any]) -> None:
    """Ensure CI has every required bulk batch baseline before tests run."""
    missing = []
    for func, sizes in REQUIRED_BULK_BASELINES.items():
        raw = baselines.get(func)
        if not isinstance(raw, dict):
            missing.append(f"{func}: all")
            continue
        for size in sizes:
            if size not in raw:
                missing.append(f"{func}: {size}")

    if missing:
        joined = ", ".join(missing)
        raise ValueError(f"Missing required gas baselines: {joined}")

def run_tests() -> str:
    print("Running gas regression tests...")
    result = subprocess.run(
        ["cargo", "test", "-p", "fluxora_stream", "--test", "gas_regression", "--", "--nocapture"],
        capture_output=True,
        text=True,
        env=build_cargo_test_env(),
    )
    # We ignore the return code here because the script handles the actual validation
    return result.stdout

def parse_measurements(output: str) -> Dict[str, Dict[str, int]]:
    measurements = {}
    # Pattern: GAS_MEASUREMENT: <function>: <size|single>: <cost>
    pattern = re.compile(r'GAS_MEASUREMENT: ([^:]+): ([^:]+): (\d+)')
    for line in output.splitlines():
        match = pattern.search(line)
        if match:
            func, size, cost = match.groups()
            if func not in measurements:
                measurements[func] = {}
            measurements[func][size] = int(cost)
    return measurements

def main():
    try:
        baselines = extract_baselines('docs/gas.md')
        validate_required_baselines(baselines)
        output = run_tests()
        measured = parse_measurements(output)

        if not measured:
            print("Error: No gas measurements found in test output.")
            sys.exit(1)

        regressions = []
        print("\nGas Cost Report:")
        print(f"{'Function':<20} | {'Size':<10} | {'Baseline':<12} | {'Measured':<12} | {'Diff %':<10} | {'Status'}")
        print("-" * 80)

        for func, sizes in measured.items():
            for size, cost in sizes.items():
                # Get baseline value.
                # Baseline entries are either:
                #   - a nested dict  {"variant": value, ...}  (e.g. batch_withdraw, keeper_cancel)
                #   - a flat integer value                     (e.g. create_stream, withdraw)
                raw = baselines.get(func)
                if isinstance(raw, dict):
                    baseline_val = raw.get(size)
                else:
                    baseline_val = raw

                if baseline_val is None:
                    print(f"{func:<20} | {size:<10} | {'N/A':<12} | {cost:<12} | {'N/A':<10} | MISSING")
                    continue

                diff = (cost - baseline_val) / baseline_val if baseline_val != 0 else 0
                status = "FAIL" if diff > 0.05 else "PASS"
                if status == "FAIL":
                    regressions.append((func, size, diff))

                print(f"{func:<20} | {size:<10} | {baseline_val:<12} | {cost:<12} | {diff:>8.2%} | {status}")

        if regressions:
            print("\nFAILED: Gas regression detected (> 5% increase) in the following functions:")
            for func, size, diff in regressions:
                print(f"- {func} ({size}): {diff:.2%}")
            sys.exit(1)
        else:
            print("\nSUCCESS: No gas regressions detected.")
            sys.exit(0)

    except Exception as e:
        print(f"Error during validation: {e}")
        sys.exit(1)

if __name__ == "__main__":
    main()
